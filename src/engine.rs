//! Suggestion pipeline.
//!
//!  main ──Job(id, pass)──▶ reader thread: UIA read → every paragraph → Harper (~5-40 ms) ──▶ main
//!                                └──LlmJob──▶ model thread: predict / revise ──▶ main
//!
//! The whole text box is checked, paragraph by paragraph. The paragraph being typed
//! is checked first; the others are checked once and cached until they change.
//!
//! Passes, all restarted by every keystroke:
//!   Quick    (~120 ms pause, or right after a word ends)  Harper + prediction +
//!            model check of the current paragraph (unfinished last sentence protected).
//!   Settle   (~400 ms pause)  the same, catching anything the quick pass skipped.
//!   Idle     (~1.2 s pause)  the paragraph is treated as a finished thought, so final
//!            punctuation may be suggested.
//!   OnDemand (shortcut)  all three modes, finished.
//!
//! Model checks of the current paragraph are *not* cancelled while the user keeps
//! typing at its end: the checked text is still a prefix of the paragraph, so its
//! fixes stay valid and appear mid-typing. They are cancelled when earlier text is
//! edited. Other work for superseded rounds is cancelled mid-generation.

use crate::config::{InferenceMode, Settings};
use crate::grammar::{Analysis, Grammar};
use crate::inference::{self, Mode, OpenAiCompat, Provider, Revision, Style};
use crate::runtime::{self, LocalServer};
use crate::text::{self, Composition, EditPlan, Run};
use crate::uia::{self, FieldContext, Reader};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

pub const WM_APP_RESULT: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    Quick,
    Settle,
    Idle,
    OnDemand,
    /// The user picked a style in the list: redo only Rephrase, in that style.
    Restyle(Style),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Correct,
    Improve,
    Rephrase,
    Next,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Correct => "Correct",
            Kind::Improve => "Improve",
            Kind::Rephrase => "Rephrase",
            Kind::Next => "Next",
        }
    }
    fn of(m: Mode) -> Kind {
        match m {
            Mode::Correct => Kind::Correct,
            Mode::Improve => Kind::Improve,
            Mode::Rephrase => Kind::Rephrase,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Suggestion {
    pub kind: Kind,
    pub runs: Vec<Run>,
    /// Edit from the caret (current paragraph only): Shift+Left then type.
    pub plan: EditPlan,
    /// The text before the caret this was computed for; re-checked before `plan` is used.
    pub basis: String,
    /// From the language model (replaces the rule checker's row of the same kind).
    pub from_model: bool,
    /// Kept on screen from an earlier pass until refreshed (main thread only).
    pub stale: bool,
    /// A single fix within the paragraph (QuillBot-style), applied on its own.
    pub issue: Option<text::Hunk>,
    /// The whole paragraph at compute time; `issue` offsets are relative to its start.
    /// Paragraphs are found again by this text, wherever the caret is.
    pub para: String,
    /// "Fix all" rows: the corrected paragraph, typed over the whole paragraph.
    pub fixed_para: Option<String>,
    /// Computed for the paragraph the caret was in.
    pub current: bool,
}

impl Suggestion {
    /// Row label in the popup.
    pub fn tag(&self) -> &'static str {
        match &self.issue {
            Some(_) if self.kind == Kind::Improve => "Clarity",
            Some(h) => h.category,
            None if self.kind == Kind::Correct => "Fix all",
            None => self.kind.label(),
        }
    }
}

/// Most individual fixes listed per paragraph; "Fix all" covers the rest.
const MAX_ISSUES: usize = 8;

/// A revision as one whole-paragraph row ("Fix all" / "Improve") plus one row per
/// individual change. `analyzed` is the text that was checked (for the current
/// paragraph: up to the caret); `para` is the whole paragraph it starts.
fn correct_items(kind: Kind, analyzed: &str, fixed: &str, para: &str, current: bool, from_model: bool) -> Vec<Suggestion> {
    let rest: String = para.chars().skip(analyzed.chars().count()).collect();
    let hunks = text::hunks(analyzed, fixed);
    let base = Suggestion {
        kind,
        runs: vec![],
        plan: EditPlan { select: 0, insert: String::new() },
        basis: analyzed.to_string(),
        from_model,
        stale: false,
        issue: None,
        para: para.to_string(),
        fixed_para: None,
        current,
    };
    let mut v = vec![];
    if hunks.len() != 1 {
        v.push(Suggestion {
            runs: text::diff_words(analyzed, fixed),
            plan: text::plan_edit(analyzed, fixed),
            fixed_para: Some(format!("{fixed}{rest}")),
            ..base.clone()
        });
    }
    for h in hunks.into_iter().take(MAX_ISSUES) {
        let single = text::apply_hunk(analyzed, &h);
        v.push(Suggestion { plan: text::plan_edit(analyzed, &single), issue: Some(h), ..base.clone() });
    }
    v
}

pub struct Update {
    pub id: u64,
    /// Some on the first update of a job: where to show the popup.
    pub anchor: Option<(isize, RECT)>,
    /// With the anchor: the paragraph the caret is in.
    pub line: Option<String>,
    pub items: Vec<Suggestion>,
    /// Field not eligible (password, excluded, not editable): hide everything.
    pub hide: bool,
    /// Message shown when the user explicitly asked but nothing is available.
    pub note: Option<String>,
    /// No more updates will follow for this id.
    pub done: bool,
    /// Still valid after newer rounds started (corrections of unchanged text).
    pub sticky: bool,
}

pub struct Job {
    pub id: u64,
    pub pass: Pass,
}

/// Another paragraph of the same text box, checked in the background.
struct ParaJob {
    text: String,
    before: String,
    after: String,
    analysis: Analysis,
}

struct LlmJob {
    id: u64,
    pass: Pass,
    comp: Composition,
    /// The whole paragraph the caret is in.
    line: String,
    analysis: Analysis,
    rules_ran: bool,
    caret_at_end: bool,
    /// Up to 300 chars before the caret, for prediction.
    context: String,
    others: Vec<ParaJob>,
}

enum LlmMsg {
    Job(LlmJob),
    Reconfigure(Settings),
}

pub struct Engine {
    pub latest: Arc<AtomicU64>,
    jobs: Sender<Job>,
    llm: Sender<LlmMsg>,
    pub results: Arc<Mutex<Vec<Update>>>,
    pub settings: Arc<Mutex<Settings>>,
}

/// Developer log, only when MARGIN_DEBUG is set (it can contain typed text).
pub fn dlog(msg: &str) {
    if std::env::var_os("MARGIN_DEBUG").is_some() {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("margin-debug.log")) {
            let _ = writeln!(f, "{:?} {msg}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() % 100_000);
        }
    }
}

fn post_result(hwnd: isize, results: &Mutex<Vec<Update>>, u: Update) {
    dlog(&format!("post id={} anchor={} sticky={} items={:?} done={}", u.id, u.anchor.is_some(), u.sticky, u.items.iter().map(|i| i.tag()).collect::<Vec<_>>(), u.done));
    results.lock().unwrap().push(u);
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_RESULT, WPARAM(0), LPARAM(0));
    }
}

fn update(id: u64) -> Update {
    Update { id, anchor: None, line: None, items: vec![], hide: false, note: None, done: false, sticky: false }
}

/// The part of `target` that further typing at its end can't change: everything up
/// to the last word boundary.
pub fn stable_prefix(target: &str) -> &str {
    if target.ends_with(char::is_whitespace) {
        return target;
    }
    match target.rfind(char::is_whitespace) {
        Some(i) => &target[..i],
        None => "",
    }
}

impl Engine {
    pub fn start(main_hwnd: isize, settings: Settings) -> Engine {
        let latest = Arc::new(AtomicU64::new(0));
        let results = Arc::new(Mutex::new(Vec::new()));
        let settings = Arc::new(Mutex::new(settings));
        let live_line = Arc::new(Mutex::new(String::new()));
        let (jtx, jrx) = channel::<Job>();
        let (ltx, lrx) = channel::<LlmMsg>();
        {
            let (latest, results, settings, ltx, live) = (latest.clone(), results.clone(), settings.clone(), ltx.clone(), live_line.clone());
            std::thread::Builder::new()
                .name("margin-reader".into())
                .spawn(move || reader_thread(main_hwnd, jrx, ltx, latest, results, settings, live))
                .unwrap();
        }
        {
            let (latest, results, s, live) = (latest.clone(), results.clone(), settings.lock().unwrap().clone(), live_line);
            std::thread::Builder::new()
                .name("margin-model".into())
                .spawn(move || model_thread(main_hwnd, lrx, latest, results, s, live))
                .unwrap();
        }
        Engine { latest, jobs: jtx, llm: ltx, results, settings }
    }

    /// Start a new suggestion round; supersedes all earlier rounds.
    pub fn request(&self, pass: Pass) -> u64 {
        let id = self.latest.fetch_add(1, SeqCst) + 1;
        let _ = self.jobs.send(Job { id, pass });
        id
    }

    /// Invalidate in-flight work without starting a new round.
    pub fn cancel(&self) {
        self.latest.fetch_add(1, SeqCst);
    }

    pub fn reconfigure(&self, s: Settings) {
        *self.settings.lock().unwrap() = s.clone();
        let _ = self.llm.send(LlmMsg::Reconfigure(s));
    }
}

/// Paragraphs other than the current one that are checked, nearest first.
const MAX_OTHER_PARAS: usize = 20;

fn reader_thread(
    hwnd: isize,
    jobs: Receiver<Job>,
    llm: Sender<LlmMsg>,
    latest: Arc<AtomicU64>,
    results: Arc<Mutex<Vec<Update>>>,
    settings: Arc<Mutex<Settings>>,
    live_line: Arc<Mutex<String>>,
) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let reader = match Reader::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("UI Automation unavailable: {e}");
            return;
        }
    };
    let mut grammar = Grammar::new(settings.lock().unwrap().dialect());

    while let Ok(mut job) = jobs.recv() {
        while let Ok(j) = jobs.try_recv() {
            job = j; // only the newest queued job matters
        }
        if job.id != latest.load(SeqCst) {
            continue;
        }
        let s = settings.lock().unwrap().clone();
        if !s.enabled {
            continue;
        }
        let on_demand = matches!(job.pass, Pass::OnDemand | Pass::Restyle(_));
        let hide = |note: Option<String>| Update { hide: true, note, done: true, ..update(job.id) };
        let ctx: FieldContext = match reader.read_focused() {
            Ok(c) => c,
            // Selecting text (e.g. a paragraph) isn't a reason to drop the underlines.
            Err(uia::Skip::NoTextAccess(why)) if why == "selection active" && !on_demand => {
                dlog("read: selection active, keeping current suggestions");
                continue;
            }
            Err(e) => {
                dlog(&format!("read {:?}: can't read field: {e:?}", job.pass));
                post_result(hwnd, &results, hide(on_demand.then(|| format!("Margin can't read this field ({e:?})"))));
                continue;
            }
        };
        if s.is_excluded(&ctx.exe, &ctx.title) {
            dlog(&format!("read: {} is excluded", ctx.exe));
            post_result(hwnd, &results, hide(None));
            continue;
        }
        grammar.set_dialect(s.dialect());

        let comp = text::composition(&ctx.before, &ctx.after);
        let before_line: String = ctx.before.rsplit(['\n', '\r']).next().unwrap_or("").to_string();
        let line = format!("{before_line}{}", ctx.after_on_line);
        *live_line.lock().unwrap() = line.clone();
        let caret_at_end = ctx.after_on_line.trim().is_empty();
        let finished = comp.ends_sentence || matches!(job.pass, Pass::Idle | Pass::OnDemand | Pass::Restyle(_));

        // The current paragraph, up to the caret (its last word may still be typed).
        // With the caret inside a paragraph (editing, not typing at the end) the whole
        // paragraph is checked below like any other, so nothing after the caret is missed.
        let analysis = if s.grammar && caret_at_end && !comp.target.trim().is_empty() { grammar.analyze(&comp.target, finished) } else { Default::default() };
        let mut items = match &analysis.fixed {
            Some(fixed) => correct_items(Kind::Correct, &comp.target, fixed, &line, true, false),
            None => vec![],
        };

        // Every other paragraph of the text box: finished text, checked whole.
        let mut full: Vec<char> = ctx.before.chars().collect();
        let caret = full.len();
        full.extend(ctx.after.chars());
        let all = uia::lines(&full);
        let ci = all.iter().position(|l| l.start <= caret && caret <= l.start + l.text.chars().count()).unwrap_or(0);
        let mut order: Vec<usize> = (0..all.len()).filter(|&i| (i != ci || !caret_at_end) && text::word_count(&all[i].text) >= 2).collect();
        order.sort_by_key(|&i| i.abs_diff(ci));
        order.truncate(MAX_OTHER_PARAS);
        let mut others = vec![];
        if s.grammar {
            for i in order {
                let t = &all[i].text;
                let a = grammar.analyze(t, true);
                if let Some(fixed) = &a.fixed {
                    items.extend(correct_items(Kind::Correct, t, fixed, t, false, false));
                }
                let before: String = all[..i].iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
                let after: String = all[i + 1..].iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
                let before: String = before.chars().rev().take(600).collect::<Vec<_>>().into_iter().rev().collect();
                let after: String = after.chars().take(300).collect();
                others.push(ParaJob { text: t.clone(), before, after, analysis: a });
            }
        }
        dlog(&format!("read {:?}: exe={} via={} paragraphs={} others={}", job.pass, ctx.exe, ctx.source, all.len(), others.len()));

        post_result(hwnd, &results, Update {
            anchor: Some((ctx.hwnd.0 as isize, ctx.caret)),
            line: Some(line.clone()),
            items,
            done: !s.ai_enabled(),
            ..update(job.id)
        });
        if s.ai_enabled() {
            let context: String = {
                let c: Vec<char> = ctx.before.chars().collect();
                c[c.len().saturating_sub(300)..].iter().collect()
            };
            let _ = llm.send(LlmMsg::Job(LlmJob {
                id: job.id,
                pass: job.pass,
                comp,
                line,
                analysis,
                rules_ran: s.grammar,
                caret_at_end,
                context,
                others,
            }));
        }
    }
}

enum Backend {
    None(String),
    Ready(Box<dyn Provider>, Option<LocalServer>),
}

fn build_backend(s: &Settings) -> Backend {
    let b = match s.inference {
        InferenceMode::Off => Backend::None("AI features are off".into()),
        InferenceMode::Remote => {
            if s.remote_url.trim().starts_with("https://") {
                let key = (!s.remote_key.trim().is_empty()).then(|| s.remote_key.trim().to_string());
                Backend::Ready(Box::new(OpenAiCompat::new(&s.remote_url, &s.remote_model, key, false)), None)
            } else {
                Backend::None("Remote endpoint must be an https:// URL".into())
            }
        }
        InferenceMode::Local => {
            let m = runtime::model(&s.local_model, s.use_gpu);
            runtime::set_status("Preparing writing model…");
            match runtime::ensure_installed(m).and_then(|(srv, model)| {
                runtime::set_status("Starting writing model…");
                LocalServer::start(&srv, &model, s.use_gpu)
            }) {
                Ok(server) => {
                    let p = OpenAiCompat::new(&format!("http://127.0.0.1:{}", server.port), "", None, true);
                    warm_up(&p);
                    Backend::Ready(Box::new(p), Some(server))
                }
                Err(e) => Backend::None(e),
            }
        }
    };
    match &b {
        Backend::Ready(p, _) => runtime::set_status(if p.is_local() { "Writing model ready (on this PC)".to_string() } else { format!("Using {}", p.label()) }),
        Backend::None(e) => runtime::set_status(e.clone()),
    }
    b
}

/// The first request per prompt is slow (measured 1-3 s): load the weights and
/// cache each mode's instructions now rather than on the user's first sentence.
pub fn warm_up(p: &dyn Provider) {
    let _ = p.complete("Thanks for");
    let r = Revision { before: "", target: "this are a test", after: "", finished: true, hints: &[], flagged: &[], mixed_language: false, foreign: &[], rules_ran: false, style: Style::Standard };
    for m in [Mode::Correct, Mode::Improve, Mode::Rephrase] {
        let _ = p.revise(m, &r, &|| false);
    }
}

/// Which modes to run for the current paragraph.
fn modes_for(s: &Settings, pass: Pass, finished: bool) -> Vec<Mode> {
    let allowed = |m: Mode| match m {
        Mode::Correct => s.grammar,
        Mode::Improve | Mode::Rephrase => s.rewriting,
    };
    let mut v: Vec<Mode> = match pass {
        Pass::OnDemand => vec![Mode::Correct, Mode::Improve, Mode::Rephrase],
        Pass::Restyle(_) => vec![Mode::Rephrase],
        // Every typing pass checks right away (measured: waiting for a longer pause
        // was most of the delay users noticed).
        Pass::Quick | Pass::Settle | Pass::Idle => {
            let m = if allowed(s.writing_mode) { s.writing_mode } else { Mode::Correct };
            vec![m]
        }
    };
    // Rephrasing half a sentence makes small models invent the rest (measured).
    v.retain(|&m| allowed(m) && (m != Mode::Rephrase || finished));
    v
}

type CacheKey = (Mode, Style, bool, String, String, String);

/// After the user accepts some individual fixes, the remaining differences between
/// the new text and the earlier correction are a subset of the earlier fixes, so
/// that correction still applies: no new model call (and no shifting suggestions).
fn still_valid(last: &Option<(String, String, String, String)>, comp: &Composition) -> Option<Vec<String>> {
    let (before, after, target, fixed) = last.as_ref()?;
    if *before != comp.before || *after != comp.after || *target == comp.target {
        return None;
    }
    let key = |h: &text::Hunk| (h.orig.trim().to_string(), h.repl.trim().to_string());
    let old: Vec<_> = text::hunks(target, fixed).iter().map(key).collect();
    let now: Vec<_> = text::hunks(&comp.target, fixed).iter().map(key).collect();
    (!now.is_empty() && now.iter().all(|h| old.contains(h))).then(|| vec![fixed.clone()])
}

/// Longest paragraph sent to the model (~350 tokens), so one cache slot
/// (3072 tokens) holds instructions, examples, context and the reply.
const MAX_TARGET_CHARS: usize = 1400;

fn model_thread(hwnd: isize, rx: Receiver<LlmMsg>, latest: Arc<AtomicU64>, results: Arc<Mutex<Vec<Update>>>, initial: Settings, live_line: Arc<Mutex<String>>) {
    let mut settings = initial;
    let mut backend = if settings.ai_enabled() { build_backend(&settings) } else { Backend::None("AI features are off".into()) };
    // In-memory only; never written to disk.
    let mut revisions: HashMap<CacheKey, Vec<String>> = HashMap::new();
    let mut predictions: HashMap<String, Option<String>> = HashMap::new();
    // Last full correction (before, after, target, fixed), to re-use after single fixes.
    let mut last_correct: Option<(String, String, String, String)> = None;

    while let Ok(msg) = rx.recv() {
        let mut job = match msg {
            LlmMsg::Reconfigure(s) => {
                let rebuild = s.inference != settings.inference
                    || s.local_model != settings.local_model
                    || s.use_gpu != settings.use_gpu
                    || s.remote_url != settings.remote_url
                    || s.remote_model != settings.remote_model
                    || s.remote_key != settings.remote_key
                    || (s.ai_enabled() && matches!(backend, Backend::None(_)));
                settings = s;
                if rebuild {
                    drop(std::mem::replace(&mut backend, Backend::None(String::new()))); // stop old server first
                    revisions.clear();
                    predictions.clear();
                    backend = if settings.ai_enabled() { build_backend(&settings) } else { Backend::None("AI features are off".into()) };
                }
                continue;
            }
            LlmMsg::Job(j) => j,
        };
        while let Ok(m) = rx.try_recv() {
            match m {
                LlmMsg::Job(j) => job = j,
                LlmMsg::Reconfigure(s) => settings = s,
            }
        }
        let id = job.id;
        let stale = || id != latest.load(SeqCst);
        if stale() {
            continue;
        }
        if let Backend::Ready(_, Some(server)) = &mut backend {
            if !server.alive() {
                backend = build_backend(&settings);
            }
        }
        let provider: &dyn Provider = match &backend {
            Backend::Ready(p, _) => p.as_ref(),
            Backend::None(why) => {
                let note = matches!(job.pass, Pass::OnDemand | Pass::Restyle(_)).then(|| if why.is_empty() { runtime::status() } else { why.clone() });
                post_result(hwnd, &results, Update { note, done: true, ..update(id) });
                continue;
            }
        };
        let comp = &job.comp;
        let explicit = matches!(job.pass, Pass::OnDemand | Pass::Restyle(_));

        // 1. Prediction (latency-sensitive, so first). Only at the end of a line,
        //    and not right after a finished sentence.
        if !explicit && settings.predictive && job.caret_at_end && !comp.ends_sentence && !job.context.trim().is_empty() {
            let ctx = job.context.clone();
            let pred = predictions.entry(ctx.clone()).or_insert_with(|| provider.complete(&ctx).ok().and_then(|raw| text::normalize_completion(&ctx, &raw))).clone();
            if predictions.len() > 256 {
                predictions.clear();
            }
            if let Some(c) = pred {
                if !stale() {
                    let tail: String = {
                        let t: Vec<char> = ctx.chars().collect();
                        let w: String = t[t.len().saturating_sub(18)..].iter().collect();
                        format!("…{}", w.trim_start())
                    };
                    post_result(hwnd, &results, Update {
                        items: vec![Suggestion {
                            kind: Kind::Next,
                            runs: vec![Run { text: tail, changed: false }, Run { text: c.clone(), changed: true }],
                            plan: EditPlan { select: 0, insert: c },
                            basis: comp.target.clone(),
                            from_model: true,
                            stale: false,
                            issue: None,
                            para: job.line.clone(),
                            fixed_para: None,
                            current: true,
                        }],
                        ..update(id)
                    });
                }
            }
        }

        // 2. The current paragraph, with the rest of the message as context.
        let pass = if job.pass == Pass::Quick && comp.ends_sentence { Pass::Settle } else { job.pass };
        let finished = comp.ends_sentence || matches!(pass, Pass::Idle | Pass::OnDemand | Pass::Restyle(_));
        let style = match job.pass {
            Pass::Restyle(st) => st,
            _ => settings.style,
        };
        // Very long paragraphs get rule fixes only: they'd overflow the model's context slot.
        let fits = comp.target.chars().count() <= MAX_TARGET_CHARS;
        let mut modes = if fits && text::word_count(&comp.target) >= 2 { modes_for(&settings, pass, finished) } else { vec![] };
        if !modes.is_empty() && !job.caret_at_end {
            if explicit {
                post_result(hwnd, &results, Update { note: Some("Move the cursor to the end of the line to check it".into()), done: true, ..update(id) });
                continue;
            }
            modes.clear(); // other paragraphs are still checked below
        }
        // Typing on at the end keeps the checked text a prefix: don't throw that work away.
        let stable = stable_prefix(&comp.target).to_string();
        let diverged = || !live_line.lock().unwrap().starts_with(stable.as_str());
        let correction_cancelled = || stale() && diverged();
        let mut posted = false;
        for mode in &modes {
            let sticky = *mode == Mode::Correct && !explicit;
            if if sticky { correction_cancelled() } else { stale() } {
                break;
            }
            let key: CacheKey = (*mode, style, finished, comp.before.clone(), comp.target.clone(), comp.after.clone());
            let reusable = (*mode == Mode::Correct).then(|| still_valid(&last_correct, comp)).flatten();
            let cands = match revisions.get(&key).cloned().or(reusable) {
                Some(v) => v,
                None => {
                    let a = &job.analysis;
                    let r = Revision {
                        before: &comp.before,
                        target: &comp.target,
                        after: &comp.after,
                        finished,
                        hints: &a.hints,
                        flagged: &a.flagged,
                        mixed_language: a.mixed_language,
                        foreign: &a.foreign,
                        rules_ran: job.rules_ran,
                        style,
                    };
                    let cancel: &dyn Fn() -> bool = if sticky { &correction_cancelled } else { &stale };
                    match inference::suggest(provider, *mode, &r, cancel) {
                        Ok(v) => {
                            if revisions.len() > 512 {
                                revisions.clear();
                            }
                            revisions.insert(key, v.clone());
                            v
                        }
                        Err(e) if e == "cancelled" => break,
                        Err(_) => {
                            revisions.insert(key, vec![]); // don't retry identical input
                            vec![]
                        }
                    }
                }
            };
            let fixed: Vec<String> = cands.iter().map(|c| text::keep_trailing_ws(&comp.target, c)).filter(|c| c.trim() != comp.target.trim()).collect();
            if *mode == Mode::Correct {
                if let Some(c) = fixed.first() {
                    last_correct = Some((comp.before.clone(), comp.after.clone(), comp.target.clone(), c.clone()));
                }
            }
            let items: Vec<Suggestion> = if *mode != Mode::Rephrase {
                fixed.first().map(|c| correct_items(Kind::of(*mode), &comp.target, c, &job.line, true, true)).unwrap_or_default()
            } else {
                fixed
                    .iter()
                    .map(|c| Suggestion {
                        kind: Kind::Rephrase,
                        runs: vec![Run { text: c.trim_end().to_string(), changed: false }],
                        plan: text::plan_edit(&comp.target, c),
                        basis: comp.target.clone(),
                        from_model: true,
                        stale: false,
                        issue: None,
                        para: job.line.clone(),
                        fixed_para: None,
                        current: true,
                    })
                    .collect()
            };
            if !sticky && stale() {
                break;
            }
            if !items.is_empty() {
                posted = true;
            }
            post_result(hwnd, &results, Update { items, sticky, ..update(id) });
        }

        // 3. Every other paragraph: cached until it changes, so this is usually free.
        if settings.grammar && !matches!(job.pass, Pass::Restyle(_)) {
            let mut checked_one = false;
            for pj in &job.others {
                if pj.text.chars().count() > MAX_TARGET_CHARS {
                    continue;
                }
                let key: CacheKey = (Mode::Correct, Style::Standard, true, String::new(), pj.text.clone(), String::new());
                let cands = match revisions.get(&key) {
                    Some(v) => v.clone(),
                    None => {
                        // A newer round goes first, but each round finishes one unchecked
                        // paragraph, so continuous typing can't starve the others.
                        if stale() && checked_one {
                            break;
                        }
                        checked_one = true;
                        let a = &pj.analysis;
                        let r = Revision {
                            before: &pj.before,
                            target: &pj.text,
                            after: &pj.after,
                            finished: true,
                            hints: &a.hints,
                            flagged: &a.flagged,
                            mixed_language: a.mixed_language,
                            foreign: &a.foreign,
                            rules_ran: job.rules_ran,
                            style: Style::Standard,
                        };
                        match inference::suggest(provider, Mode::Correct, &r, &|| false) {
                            Ok(v) => {
                                revisions.insert(key, v.clone());
                                v
                            }
                            Err(e) if e == "cancelled" => break,
                            Err(_) => {
                                revisions.insert(key, vec![]);
                                vec![]
                            }
                        }
                    }
                };
                let items: Vec<Suggestion> = cands
                    .first()
                    .map(|c| text::keep_trailing_ws(&pj.text, c))
                    .filter(|c| c.trim() != pj.text.trim())
                    .map(|c| correct_items(Kind::Correct, &pj.text, &c, &pj.text, false, true))
                    .unwrap_or_default();
                if !items.is_empty() {
                    post_result(hwnd, &results, Update { items, sticky: true, ..update(id) });
                }
            }
        }
        if !stale() {
            let note = (explicit && !posted).then(|| "Looks good. No suggestions.".to_string());
            post_result(hwnd, &results, Update { note, done: true, ..update(id) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_prefix_drops_the_word_being_typed() {
        assert_eq!(stable_prefix("I dont think tha"), "I dont think");
        assert_eq!(stable_prefix("I dont think "), "I dont think ");
        assert_eq!(stable_prefix("word"), "");
    }

    #[test]
    fn correct_items_for_a_whole_paragraph() {
        let items = correct_items(Kind::Correct, "i dont know", "I don't know.", "i dont know", false, false);
        let fix_all = items.iter().find(|i| i.issue.is_none()).unwrap();
        assert_eq!(fix_all.fixed_para.as_deref(), Some("I don't know."));
        assert!(items.iter().filter(|i| i.issue.is_some()).all(|i| i.para == "i dont know" && !i.current));
        // Current paragraph with text after the caret: the rest is kept.
        let items = correct_items(Kind::Correct, "i dont ", "I don't ", "i dont know", true, false);
        let fix_all = items.iter().find(|i| i.issue.is_none()).unwrap();
        assert_eq!(fix_all.fixed_para.as_deref(), Some("I don't know"));
        assert!(items.iter().all(|i| i.current && i.para == "i dont know"));
    }
}
