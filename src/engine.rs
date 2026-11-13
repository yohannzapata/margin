//! Suggestion pipeline.
//!
//!  main ──Job(id, pass)──▶ reader thread: UIA read → Composition → Harper (~5-40 ms) ──▶ main
//!                                └──LlmJob──▶ model thread: predict / revise ──▶ main
//!
//! Passes, all restarted by every keystroke:
//!   Quick    (~180 ms pause)  Harper fixes + next-word prediction. Cheap.
//!   Settle   (~700 ms pause, or right after . ? !)  model revision of the whole
//!            paragraph in the chosen mode; an unfinished last sentence is protected.
//!   Idle     (~2 s pause)  the paragraph is treated as a finished thought, so final
//!            punctuation may be suggested.
//!   OnDemand (shortcut)  all three modes, finished.
//! Every job carries an id. Typing advances the id; work for older ids is
//! cancelled mid-generation and never reaches the screen.

use crate::config::{InferenceMode, Settings};
use crate::grammar::Grammar;
use crate::inference::{self, Mode, OpenAiCompat, Provider, Revision, Style};
use crate::runtime::{self, LocalServer};
use crate::text::{self, Composition, EditPlan, Run};
use crate::uia::{FieldContext, Reader};
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
    pub plan: EditPlan,
    /// The paragraph text this was computed for; re-checked before applying.
    pub basis: String,
    /// From the language model (replaces the rule checker's row of the same kind).
    pub from_model: bool,
    /// Kept on screen from an earlier pass until refreshed (main thread only).
    pub stale: bool,
    /// A single fix within the paragraph (QuillBot-style), applied on its own.
    pub issue: Option<text::Hunk>,
    /// The whole current line at compute time (text before + after the caret);
    /// `issue` offsets are relative to its start.
    pub para: String,
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

/// Most individual fixes listed at once; "Fix all" covers the rest.
const MAX_ISSUES: usize = 6;

/// A revision as one whole-text row ("Fix all" / "Improve") plus one row per individual change.
fn correct_items(kind: Kind, target: &str, fixed: &str, after_on_line: &str, from_model: bool) -> Vec<Suggestion> {
    let para = format!("{target}{after_on_line}");
    let hunks = text::hunks(target, fixed);
    let mut v = vec![];
    if hunks.len() != 1 {
        v.push(Suggestion {
            kind,
            runs: text::diff_words(target, fixed),
            plan: text::plan_edit(target, fixed),
            basis: target.to_string(),
            from_model,
            stale: false,
            issue: None,
            para: para.clone(),
        });
    }
    for h in hunks.into_iter().take(MAX_ISSUES) {
        let single = text::apply_hunk(target, &h);
        v.push(Suggestion {
            kind,
            runs: vec![],
            plan: text::plan_edit(target, &single),
            basis: target.to_string(),
            from_model,
            stale: false,
            issue: Some(h),
            para: para.clone(),
        });
    }
    v
}

pub struct Update {
    pub id: u64,
    /// Some on the first update of a job: where to show the popup.
    pub anchor: Option<(isize, RECT)>,
    pub items: Vec<Suggestion>,
    /// Field not eligible (password, excluded, not editable): hide everything.
    pub hide: bool,
    /// Message shown when the user explicitly asked but nothing is available.
    pub note: Option<String>,
    /// No more updates will follow for this id.
    pub done: bool,
    /// A re-evaluation of unchanged text: keep current rows until replaced.
    pub refresh: bool,
}

pub struct Job {
    pub id: u64,
    pub pass: Pass,
}

struct LlmJob {
    id: u64,
    pass: Pass,
    comp: Composition,
    hints: Vec<String>,
    flagged: Vec<(String, String)>,
    mixed_language: bool,
    foreign: Vec<String>,
    rules_ran: bool,
    caret_at_end: bool,
    /// Up to 300 chars before the caret, for prediction.
    context: String,
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
    dlog(&format!("post id={} anchor={} items={:?} done={}", u.id, u.anchor.is_some(), u.items.iter().map(|i| i.tag()).collect::<Vec<_>>(), u.done));
    results.lock().unwrap().push(u);
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_RESULT, WPARAM(0), LPARAM(0));
    }
}

fn update(id: u64) -> Update {
    Update { id, anchor: None, items: vec![], hide: false, note: None, done: false, refresh: false }
}

impl Engine {
    pub fn start(main_hwnd: isize, settings: Settings) -> Engine {
        let latest = Arc::new(AtomicU64::new(0));
        let results = Arc::new(Mutex::new(Vec::new()));
        let settings = Arc::new(Mutex::new(settings));
        let (jtx, jrx) = channel::<Job>();
        let (ltx, lrx) = channel::<LlmMsg>();
        {
            let (latest, results, settings, ltx) = (latest.clone(), results.clone(), settings.clone(), ltx.clone());
            std::thread::Builder::new()
                .name("margin-reader".into())
                .spawn(move || reader_thread(main_hwnd, jrx, ltx, latest, results, settings))
                .unwrap();
        }
        {
            let (latest, results, s) = (latest.clone(), results.clone(), settings.lock().unwrap().clone());
            std::thread::Builder::new()
                .name("margin-model".into())
                .spawn(move || model_thread(main_hwnd, lrx, latest, results, s))
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

fn reader_thread(hwnd: isize, jobs: Receiver<Job>, llm: Sender<LlmMsg>, latest: Arc<AtomicU64>, results: Arc<Mutex<Vec<Update>>>, settings: Arc<Mutex<Settings>>) {
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
        dlog(&format!(
            "read {:?}: exe={} via={} caret=({},{},{},{}) before_len={} after_on_line={:?}",
            job.pass, ctx.exe, ctx.source, ctx.caret.left, ctx.caret.top, ctx.caret.right, ctx.caret.bottom,
            ctx.before.chars().count(), ctx.after_on_line.chars().take(20).collect::<String>()
        ));
        grammar.set_dialect(s.dialect());

        let comp = text::composition(&ctx.before, &ctx.after);
        let caret_at_end = ctx.after_on_line.trim().is_empty();
        let finished = comp.ends_sentence || matches!(job.pass, Pass::Idle | Pass::OnDemand | Pass::Restyle(_));
        let analysis = if s.grammar && !comp.target.trim().is_empty() { grammar.analyze(&comp.target, finished) } else { Default::default() };
        let items = match &analysis.fixed {
            Some(fixed) => correct_items(Kind::Correct, &comp.target, fixed, &ctx.after_on_line, false),
            None => vec![],
        };
        let refresh = matches!(job.pass, Pass::Settle | Pass::Idle | Pass::Restyle(_));
        post_result(hwnd, &results, Update {
            anchor: Some((ctx.hwnd.0 as isize, ctx.caret)),
            items,
            done: !s.ai_enabled(),
            refresh,
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
                hints: analysis.hints,
                flagged: analysis.flagged,
                mixed_language: analysis.mixed_language,
                foreign: analysis.foreign,
                rules_ran: s.grammar,
                caret_at_end,
                context,
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

/// Which modes to run for a job.
fn modes_for(s: &Settings, pass: Pass, finished: bool) -> Vec<Mode> {
    let allowed = |m: Mode| match m {
        Mode::Correct => s.grammar,
        Mode::Improve | Mode::Rephrase => s.rewriting,
    };
    let mut v: Vec<Mode> = match pass {
        Pass::Quick => vec![],
        Pass::OnDemand => vec![Mode::Correct, Mode::Improve, Mode::Rephrase],
        Pass::Restyle(_) => vec![Mode::Rephrase],
        Pass::Settle | Pass::Idle => {
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

fn model_thread(hwnd: isize, rx: Receiver<LlmMsg>, latest: Arc<AtomicU64>, results: Arc<Mutex<Vec<Update>>>, initial: Settings) {
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

        // 1. Prediction (latency-sensitive, so first). Only at the end of a line,
        //    and not right after a finished sentence.
        dlog(&format!(
            "job {} {:?} caret_at_end={} ends_sentence={} predictive={} ctx_tail={:?}",
            id, job.pass, job.caret_at_end, comp.ends_sentence, settings.predictive,
            job.context.chars().rev().take(20).collect::<String>().chars().rev().collect::<String>()
        ));
        // Every typing pass predicts (cached, so repeats are free): when the model is busy
        // or still loading, queued passes are coalesced and the quick one may be dropped.
        if !matches!(job.pass, Pass::OnDemand | Pass::Restyle(_)) && settings.predictive && job.caret_at_end && !comp.ends_sentence && !job.context.trim().is_empty() {
            let ctx = job.context.clone();
            let pred = predictions
                .entry(ctx.clone())
                .or_insert_with(|| {
                    let raw = provider.complete(&ctx);
                    dlog(&format!("complete -> {raw:?}"));
                    raw.ok().and_then(|raw| text::normalize_completion(&ctx, &raw))
                })
                .clone();
            dlog(&format!("prediction {pred:?}"));
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
                            para: comp.target.clone(),
                        }],
                        ..update(id)
                    });
                }
            }
        }

        // 2. Revisions of the whole paragraph, with the rest of the message as context.
        // A just-finished sentence is worth revising immediately, without waiting to settle.
        let pass = if job.pass == Pass::Quick && comp.ends_sentence { Pass::Settle } else { job.pass };
        let finished = comp.ends_sentence || matches!(pass, Pass::Idle | Pass::OnDemand | Pass::Restyle(_));
        let style = match job.pass {
            Pass::Restyle(st) => st,
            _ => settings.style,
        };
        // Very long paragraphs get rule fixes only: they'd overflow the model's context slot.
        let fits = comp.target.chars().count() <= MAX_TARGET_CHARS;
        let modes = if fits && text::word_count(&comp.target) >= 2 { modes_for(&settings, pass, finished) } else { vec![] };
        if !modes.is_empty() && !job.caret_at_end {
            if matches!(job.pass, Pass::OnDemand | Pass::Restyle(_)) {
                post_result(hwnd, &results, Update { note: Some("Move the cursor to the end of the line to check it".into()), done: true, ..update(id) });
            }
            continue;
        }
        let mut posted = false;
        for mode in &modes {
            if stale() {
                break;
            }
            let key: CacheKey = (*mode, style, finished, comp.before.clone(), comp.target.clone(), comp.after.clone());
            let reusable = (*mode == Mode::Correct).then(|| still_valid(&last_correct, comp)).flatten();
            let cands = match revisions.get(&key).cloned().or(reusable) {
                Some(v) => v,
                None => {
                    let r = Revision {
                        before: &comp.before,
                        target: &comp.target,
                        after: &comp.after,
                        finished,
                        hints: &job.hints,
                        flagged: &job.flagged,
                        mixed_language: job.mixed_language,
                        foreign: &job.foreign,
                        rules_ran: job.rules_ran,
                        style,
                    };
                    match inference::suggest(provider, *mode, &r, &stale) {
                        Ok(v) => {
                            if revisions.len() > 256 {
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
                fixed.first().map(|c| correct_items(Kind::of(*mode), &comp.target, c, "", true)).unwrap_or_default()
            } else {
                fixed
                    .iter()
                    .map(|c| Suggestion {
                        kind: Kind::of(*mode),
                        runs: if *mode == Mode::Rephrase { vec![Run { text: c.trim_end().to_string(), changed: false }] } else { text::diff_words(&comp.target, c) },
                        plan: text::plan_edit(&comp.target, c),
                        basis: comp.target.clone(),
                        from_model: true,
                        stale: false,
                        issue: None,
                        para: comp.target.clone(),
                    })
                    .collect()
            };
            if stale() {
                break;
            }
            if !items.is_empty() {
                posted = true;
            }
            post_result(hwnd, &results, Update { items, ..update(id) });
        }
        if !stale() {
            let note = (matches!(job.pass, Pass::OnDemand | Pass::Restyle(_)) && !posted).then(|| "Looks good. No suggestions.".to_string());
            post_result(hwnd, &results, Update { note, done: true, ..update(id) });
        }
    }
}
