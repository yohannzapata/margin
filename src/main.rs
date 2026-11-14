#![cfg_attr(not(test), windows_subsystem = "windows")]
//! Margin: a system-wide writing assistant for Windows.

use margin::*;

use config::{InferenceMode, Settings};
use engine::{Engine, Kind, Pass};
use hook::*;
use std::sync::atomic::Ordering::Relaxed;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

const TIMER_DEBOUNCE: usize = 1;
const TIMER_WATCH: usize = 2;
const TIMER_SETTLE: usize = 3;
const TIMER_IDLE: usize = 4;
const TIMER_HOVER: usize = 5;
const TIMER_REMEASURE: usize = 6;
/// Pause before the quick (rule checker + prediction) pass. Measured: anything longer
/// meant underlines never appeared while typing at a normal speed.
const QUICK_MS: u32 = 120;
/// Right after a word ends (space, punctuation) the quick pass runs at once.
const WORD_END_MS: u32 = 15;
/// Pause after which the paragraph is revised by the model.
const SETTLE_MS: u32 = 400;
/// Pause after which the paragraph is treated as a finished thought.
const IDLE_MS: u32 = 1200;

struct App {
    inst: HINSTANCE,
    hwnd: HWND,
    engine: Engine,
    round: u64,
    anchor: Option<(HWND, RECT)>,
    on_demand: bool,
    taskbar_created: u32,
    /// Re-reads the field right before applying a suggestion.
    reader: Option<uia::Reader>,
    /// All suggestions for the current round. The popup shows these, or a hover card.
    list: Vec<engine::Suggestion>,
    /// Individual fixes currently underlined (indices match the overlay's hits).
    issues: Vec<engine::Suggestion>,
    /// The issue whose card is showing because the mouse is over its underline.
    hover: Option<usize>,
    /// Which fix the hover card is for (paragraph, start): underlines are renumbered
    /// when results arrive, so the card must not follow the number.
    hover_key: Option<(String, usize)>,
    /// When the mouse left both the underline and its card.
    hover_left: Option<std::time::Instant>,
    /// Predicted next words, shown as grey text after the caret (inline style).
    ghost: Option<engine::Suggestion>,
    /// The user asked for the full list (shortcut): show the popup even in inline style.
    show_list: bool,
    /// The current round only redoes Rephrase in a new style (other rows stay).
    restyle: bool,
    /// The paragraph the caret was in at the last pass.
    line: String,
}

static mut APP: Option<App> = None;

#[allow(static_mut_refs)]
fn app() -> &'static mut App {
    unsafe { APP.as_mut().unwrap() }
}

fn settings() -> Settings {
    app().engine.settings.lock().unwrap().clone()
}

fn apply_settings_to_hook(s: &Settings) {
    ENABLED.store(s.enabled, Relaxed);
    ON_DEMAND.store(parse_shortcut(&s.on_demand_shortcut).unwrap_or(0), Relaxed);
    *theme::BADGE.lock().unwrap() = match s.inference {
        InferenceMode::Local => "On this PC".into(),
        InferenceMode::Off => "Rules only · on this PC".into(),
        InferenceMode::Remote => {
            let host = s.remote_url.split("//").nth(1).unwrap_or("").split('/').next().unwrap_or("").to_string();
            format!("Sent to {host}")
        }
    };
}

fn tooltip(s: &Settings) -> String {
    if !s.enabled {
        return "Margin: paused".into();
    }
    let st = runtime::status();
    if st.is_empty() { "Margin".into() } else { format!("Margin: {st}") }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--probe") {
        return probe(std::env::args().nth(2).unwrap_or_else(|| "margin-probe.txt".into()));
    }
    unsafe {
        // One instance per user session.
        let _mutex = CreateMutexW(None, true, w!("Local\\MarginWritingAssistant"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let inst: HINSTANCE = GetModuleHandleW(None).unwrap().into();

        let wc = WNDCLASSW { lpfnWndProc: Some(main_proc), hInstance: inst, lpszClassName: w!("MarginMain"), ..Default::default() };
        RegisterClassW(&wc);
        popup::register(inst);
        settings_ui::register(inst);

        let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), w!("MarginMain"), w!("Margin"), WS_OVERLAPPED, 0, 0, 0, 0, None, None, Some(inst), None).unwrap();
        MAIN_HWND.store(hwnd.0 as isize, Relaxed);

        let first_run = !config::dir().join("settings.json").exists();
        let s = config::load();
        if first_run {
            let _ = config::save(&s);
        }
        apply_settings_to_hook(&s);
        let engine = Engine::start(hwnd.0 as isize, s.clone());
        APP = Some(App {
            inst,
            hwnd,
            engine,
            round: 0,
            anchor: None,
            on_demand: false,
            reader: uia::Reader::new().ok(),
            list: vec![],
            issues: vec![],
            hover: None,
            hover_key: None,
            hover_left: None,
            ghost: None,
            show_list: false,
            restyle: false,
            line: String::new(),
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
        });
        popup::POPUP = Some(popup::Popup::create(inst));
        overlay::register(inst);
        overlay::OVERLAY = Some(overlay::Overlay::create(inst));
        tray::add(hwnd, &tooltip(&s));
        hook::spawn();
        SetTimer(Some(hwnd), TIMER_WATCH, 250, None);
        if first_run {
            settings_ui::open(inst, hwnd, s);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if let Some(sw) = settings_ui::hwnd() {
                if IsDialogMessageW(sw, &msg).as_bool() {
                    continue;
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        tray::remove(hwnd);
    }
}

fn on_typing(flags: usize) {
    let p = popup::get();
    if p.visible() {
        p.hide();
    }
    let a = app();
    a.hover = None;
    a.hover_key = None;
    a.hover_left = None;
    if flags & KEY_EDITING != 0 {
        // Backspace, Enter, arrows, paste…: text may move anywhere, so hide underlines.
        clear_underlines(a);
        a.list.clear();
    } else {
        // Typing a character: words before the caret stay put, so their underlines stay
        // on screen. Only the grey prediction (at the caret) goes away.
        a.ghost = None;
        overlay::get().drop_ghost();
        overlay::get().set_hover(None);
        sync_tab();
    }
    a.show_list = false;
    a.restyle = false;
    a.engine.cancel();
    a.on_demand = false;
    let quick = if flags & KEY_WORD_END != 0 { WORD_END_MS } else { settings().debounce_ms.clamp(60, 1000).min(QUICK_MS) };
    unsafe {
        SetTimer(Some(a.hwnd), TIMER_DEBOUNCE, quick, None);
        SetTimer(Some(a.hwnd), TIMER_SETTLE, SETTLE_MS, None);
        SetTimer(Some(a.hwnd), TIMER_IDLE, IDLE_MS, None);
        // Don't re-measure mid-typing (the line keeps changing); the next pass redraws.
        if overlay::get().visible() {
            SetTimer(Some(a.hwnd), TIMER_REMEASURE, 900, None);
        }
    }
}

fn on_action(action: usize) {
    let a = app();
    let p = popup::get();
    let style_index = action >> 8;
    let action = action & 0xFF;
    match action {
        ACT_STYLE | ACT_STYLE_PREV | ACT_STYLE_NEXT if p.visible() && p.styles.is_some() => {
            let n = inference::Style::ALL.len();
            let cur = inference::Style::ALL.iter().position(|s| Some(*s) == p.styles).unwrap_or(0);
            let next = match action {
                ACT_STYLE => style_index.min(n - 1),
                ACT_STYLE_PREV => (cur + n - 1) % n,
                _ => (cur + 1) % n,
            };
            let style = inference::Style::ALL[next];
            // Remember the choice; it becomes the default for next time.
            let mut s = settings();
            s.style = style;
            let _ = config::save(&s);
            a.engine.reconfigure(s);
            p.styles = Some(style);
            // Only the Rephrase rows change; show a note while they're rewritten.
            a.list.retain(|e| e.kind != Kind::Rephrase);
            p.items.retain(|e| e.kind != Kind::Rephrase);
            p.selected = 0;
            p.busy = true;
            a.restyle = true;
            a.on_demand = true;
            a.round = a.engine.request(Pass::Restyle(style));
            if let Some((_, caret)) = a.anchor {
                p.show_at(caret);
            }
        }
        ACT_ACCEPT => {
            let from_popup = if p.visible() { p.items.get(p.selected).cloned() } else { None };
            engine::dlog(&format!("accept requested: popup_visible={} selected={} items={}", p.visible(), p.selected, p.items.len()));
            let Some(item) = from_popup.or_else(|| a.ghost.clone()) else { return };
            p.hide();
            a.engine.cancel();
            stop_pass_timers(a.hwnd);
            clear_underlines(a);
            let Some(ft) = a.reader.as_ref().and_then(|r| r.field_text()) else {
                engine::dlog("accept: field unreadable");
                return;
            };
            let before: String = ft.full[..ft.caret].iter().collect();
            let caret_unmoved = text::composition(&before, "").target == item.basis;
            let reader = a.reader.as_ref().unwrap();
            engine::dlog(&format!(
                "accept {:?} issue={:?} para={:?} basis_len={} caret={} caret_unmoved={caret_unmoved} delta={:?}",
                item.kind, item.issue.as_ref().map(|h| (h.start, h.end, h.orig.clone(), h.repl.clone())), item.para, item.basis.chars().count(),
                ft.caret, para_delta(&ft, &item)
            ));
            let applied = match (&item.issue, &item.fixed_para) {
                // One fix in the paragraph being typed, caret where it was: Shift+Left from
                // the caret and retype (the most widely compatible method; caret ends where it was).
                (Some(_), _) if item.current && caret_unmoved => inject::apply(&item.plan),
                // Anywhere else: find its paragraph by text, select exactly that word (verified
                // to still read the same), type the fix, and put the caret back.
                (Some(h), _) => match para_delta(&ft, &item) {
                    Some(d) if reader.select_rel(&ft, d + h.start as isize, d + h.end as isize, &h.orig) => {
                        let at = |o: usize| (ft.caret as isize + d + o as isize) as usize;
                        inject::replace_and_restore(&ft.full, ft.caret, at(h.start), at(h.end), &h.repl)
                    }
                    _ => false,
                },
                // "Fix all" for a paragraph the caret isn't at the end of: retype that paragraph.
                (None, Some(fixed)) if !(item.current && caret_unmoved) => {
                    let para = item.para.trim_end();
                    match ft.locate(para) {
                        Some(d) if reader.select_rel(&ft, d, d + para.chars().count() as isize, para) => {
                            let start = (ft.caret as isize + d) as usize;
                            inject::replace_and_restore(&ft.full, ft.caret, start, start + para.chars().count(), fixed.trim_end())
                        }
                        _ => false,
                    }
                }
                // Whole-text rows at the caret (Fix all, Improve, Rephrase, prediction).
                _ if caret_unmoved => inject::apply(&item.plan),
                // The text changed since the suggestion was made: applying it could corrupt it.
                _ => false,
            };
            if !applied {
                return;
            }
            if item.issue.is_some() || item.fixed_para.is_some() {
                // Re-check (earlier corrections are reused, so this is quick).
                unsafe {
                    SetTimer(Some(a.hwnd), TIMER_DEBOUNCE, 150, None);
                }
            }
            // Chain predictions like a phone keyboard: Tab, Tab, Tab…
            if item.kind == Kind::Next {
                unsafe {
                    SetTimer(Some(a.hwnd), TIMER_DEBOUNCE, 250, None);
                }
            }
        }
        ACT_NEXT | ACT_PREV if !p.items.is_empty() => {
            let n = p.items.len();
            p.selected = if action == ACT_NEXT { (p.selected + 1) % n } else { (p.selected + n - 1) % n };
            p.refresh();
        }
        ACT_DISMISS => {
            p.hide();
            a.engine.cancel();
            stop_pass_timers(a.hwnd);
            clear_underlines(a);
        }
        ACT_ON_DEMAND => {
            p.hide();
            stop_pass_timers(a.hwnd);
            a.on_demand = true;
            a.show_list = true;
            a.restyle = false;
            a.round = a.engine.request(Pass::OnDemand);
        }
        _ => {}
    }
}

fn on_results() {
    let a = app();
    let p = popup::get();
    let updates: Vec<engine::Update> = std::mem::take(&mut *a.engine.results.lock().unwrap());
    let (mut changed, mut done, mut note) = (false, false, None);
    for u in updates {
        let current_round = u.id == a.round;
        // Corrections of unchanged text stay valid across rounds; everything else is superseded.
        if !current_round && !u.sticky {
            continue;
        }
        if u.hide {
            if current_round {
                p.hide();
                a.list.clear();
                clear_underlines(a);
                if let (Some(n), Some(anchor)) = (u.note, fallback_anchor()) {
                    p.note = Some(n);
                    p.owner = anchor.0;
                    p.show_at(anchor.1);
                }
            }
            continue;
        }
        changed = true;
        if let (true, Some((h, caret))) = (current_round, u.anchor) {
            a.anchor = Some((HWND(h as _), caret));
            if let Some(l) = u.line {
                a.line = l;
            }
            // Re-evaluation: keep rows on screen until fresher ones replace them.
            // A restyle only replaces Rephrase rows.
            let restyle = a.restyle;
            for it in a.list.iter_mut().filter(|i| !restyle || i.kind == Kind::Rephrase) {
                it.stale = true;
            }
            p.owner = HWND(h as _);
            p.note = a.on_demand.then(|| "Checking…".to_string());
        }
        merge(&mut a.list, u.items);
        if current_round {
            if u.done {
                a.list.retain(|e| !e.stale);
                done = true;
                a.on_demand = false;
            }
            note = note.or(u.note);
        }
    }
    if !changed {
        return;
    }
    let list = &mut a.list;
    // Fix all, individual fixes in text order (current paragraph first), Improve, Rephrase, Next.
    // Per kind: the current paragraph first, its "Fix all" before its individual fixes.
    let key = |x: &engine::Suggestion| (x.kind, !x.current, x.stale, x.issue.is_some(), x.para.clone(), x.issue.as_ref().map(|h| h.start).unwrap_or(0));
    list.sort_by(|x, y| key(x).cmp(&key(y)));
    let inline = settings().inline && !a.show_list;
    a.ghost = if inline { a.list.iter().find(|i| i.kind == Kind::Next && !i.stale).cloned() } else { None };
    // A hover card the user is looking at is never replaced by a late result.
    if a.hover.is_none() && inline {
        if p.visible() {
            p.hide();
        }
    } else if a.hover.is_none() {
        p.compact = false;
        // Style buttons whenever the list offers (or can offer) Rephrase.
        p.styles = (a.show_list && settings().rewriting).then(|| settings().style);
        // The list is about the paragraph being written; underlines cover the rest.
        p.items = a.list.iter().filter(|e| e.current).take(9).cloned().collect();
        p.selected = p.selected.min(p.items.len().saturating_sub(1));
        p.busy = !done;
        if !p.items.is_empty() {
            p.note = None;
        } else if let Some(n) = note {
            p.note = Some(n);
        } else if done && p.note.is_some() {
            p.note = Some("Looks good. No suggestions.".into());
        }
        if let Some((_, caret)) = a.anchor {
            p.show_at(caret);
        }
    }
    update_underlines(a);
}

/// Rows replace older rows of the same kind for the same paragraph ("group"). The
/// paragraph being typed is one group however much it grows. A fresh model row beats
/// the rule checker's; stale rows stay visible until something replaces them.
fn merge(list: &mut Vec<engine::Suggestion>, items: Vec<engine::Suggestion>) {
    let group = |e: &engine::Suggestion| (e.kind, if e.current { String::new() } else { e.para.clone() });
    let mut groups: Vec<(Kind, String)> = items.iter().map(group).collect();
    groups.dedup();
    let from_model = items.first().is_some_and(|i| i.from_model);
    // Checks of the paragraph being typed keep running while the user types, so a check
    // of an older (shorter) version can finish after a newer one: never let it win.
    if from_model && items.iter().all(|i| i.current) {
        let incoming = items[0].para.trim_end().to_string();
        let newer_exists = list.iter().any(|e| {
            e.current && e.from_model && !e.stale && e.kind == items[0].kind && e.para.trim_end().len() > incoming.len() && e.para.starts_with(engine::stable_prefix(&incoming))
        });
        if newer_exists {
            return;
        }
    }
    let mut skip: Vec<(Kind, String)> = vec![];
    for g in &groups {
        if from_model {
            list.retain(|e| group(e) != *g);
        } else if list.iter().any(|e| group(e) == *g && e.from_model && !e.stale) {
            skip.push(g.clone());
        } else {
            list.retain(|e| !(group(e) == *g && e.stale && !e.from_model));
        }
    }
    for it in items {
        if skip.contains(&group(&it)) {
            continue;
        }
        let dup = list.iter().any(|e| e.plan == it.plan && e.issue == it.issue && e.para == it.para);
        if !dup && (!it.plan.insert.is_empty() || it.plan.select > 0 || it.issue.is_some() || it.fixed_para.is_some()) {
            list.push(it);
        }
    }
}

/// Where `item`'s paragraph starts relative to the caret. Found by its text; the
/// paragraph being typed may have grown since, so a row computed on an earlier
/// version still counts if that version is a prefix of the current line (fixes in
/// the part still being typed are dropped).
fn para_delta(ft: &uia::FieldText, item: &engine::Suggestion) -> Option<isize> {
    if let Some(d) = ft.locate(&item.para) {
        return Some(d);
    }
    if !item.current {
        return None;
    }
    let lines = uia::lines(&ft.full);
    let line = &lines[ft.caret_line()?];
    let stable = engine::stable_prefix(&item.para);
    let fits = item.issue.as_ref().is_some_and(|h| h.end <= stable.chars().count());
    (fits && !stable.is_empty() && line.text.starts_with(stable)).then(|| line.start as isize - ft.caret as isize)
}

fn hover_key(it: &engine::Suggestion) -> (String, usize) {
    (it.para.clone(), it.issue.as_ref().map(|h| h.start).unwrap_or(usize::MAX))
}

/// Most underlines drawn at once (each costs a UI Automation round trip).
const MAX_UNDERLINES: usize = 40;

/// Underline every individual fix in the whole text box.
fn update_underlines(a: &mut App) {
    let ghost = a.ghost.as_ref().zip(a.anchor).map(|(g, (_, caret))| (caret, g.plan.insert.clone()));
    let owner = a.anchor.map(|x| x.0).unwrap_or_default();
    let Some(ft) = a.reader.as_ref().and_then(|r| r.field_text()) else {
        clear_underlines(a);
        return;
    };
    // Locate each fix; drop rows whose paragraph is gone (edited or deleted).
    let mut located: Vec<(isize, isize, engine::Suggestion)> = vec![];
    a.list.retain(|it| {
        let Some(h) = &it.issue else {
            // Whole-paragraph rows elsewhere must still find their paragraph.
            return it.current || it.fixed_para.is_none() || ft.locate(&it.para).is_some();
        };
        match para_delta(&ft, it) {
            Some(d) => {
                located.push((d + h.start as isize, d + h.end as isize, it.clone()));
                true
            }
            None => false,
        }
    });
    // Where model and rule fixes overlap, the model's (which saw the whole message) wins.
    located.sort_by_key(|(s, _, it)| (!it.from_model, it.stale, s.abs()));
    let mut kept: Vec<(isize, isize, engine::Suggestion)> = vec![];
    for (s, e, it) in located {
        if kept.len() < MAX_UNDERLINES && !kept.iter().any(|(ks, ke, _)| s < *ke && *ks < e) {
            kept.push((s, e, it));
        }
    }
    if kept.is_empty() {
        overlay::get().hide();
        a.issues.clear();
        if ghost.is_some() {
            overlay::get().show(owner, vec![], ghost);
        }
        sync_tab();
        return;
    }
    let spans: Vec<(isize, isize, String)> = kept.iter().map(|(s, e, it)| (*s, *e, it.issue.as_ref().unwrap().orig.clone())).collect();
    let rects = a.reader.as_ref().map(|r| r.rects_rel(&spans)).unwrap_or_default();
    engine::dlog(&format!("underline {} fix(es), {} measured", spans.len(), rects.iter().filter(|r| !r.is_empty()).count()));
    let issues: Vec<engine::Suggestion> = kept.into_iter().map(|k| k.2).collect();
    let color = |i: usize| if issues[i].kind == Kind::Improve { overlay::BLUE } else { overlay::RED };
    let hits: Vec<(usize, RECT, u32)> = rects.into_iter().enumerate().flat_map(|(i, rs)| rs.into_iter().map(move |r| (i, r, color(i)))).collect();
    a.issues = issues;
    overlay::get().show(owner, hits, ghost);
    // Keep the hovered fix highlighted under its new number.
    a.hover = a.hover_key.as_ref().and_then(|k| a.issues.iter().position(|it| hover_key(it) == *k));
    overlay::get().set_hover(a.hover);
    sync_tab();
    unsafe {
        SetTimer(Some(a.hwnd), TIMER_HOVER, 80, None);
        SetTimer(Some(a.hwnd), TIMER_REMEASURE, 700, None);
    }
}

/// Tab/Esc are captured only while a suggestion is actually offered:
/// a visible popup/pill, or a grey prediction after the caret.
fn sync_tab() {
    let p = popup::get();
    let offered = (p.visible() && !p.items.is_empty()) || overlay::get().ghost.is_some();
    hook::POPUP_VISIBLE.store(offered, Relaxed);
}

fn clear_underlines(a: &mut App) {
    overlay::get().hide();
    a.issues.clear();
    a.ghost = None;
    sync_tab();
    a.hover = None;
    a.hover_key = None;
    a.hover_left = None;
    unsafe {
        let _ = KillTimer(Some(a.hwnd), TIMER_HOVER);
        let _ = KillTimer(Some(a.hwnd), TIMER_REMEASURE);
    }
}

/// Hovering an underline shows a card with just that fix (and Fix all).
fn on_hover() {
    let a = app();
    let p = popup::get();
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    let over_card = p.visible() && unsafe {
        let mut r = RECT::default();
        GetWindowRect(p.hwnd, &mut r).is_ok() && pt.x >= r.left && pt.x <= r.right && pt.y >= r.top && pt.y <= r.bottom
    };
    // The card sits on the line below the word: while the mouse is on the card, the
    // underlines underneath it don't count (otherwise the card swaps under the click).
    if over_card {
        a.hover_left = None;
        return;
    }
    match overlay::get().hit(pt) {
        Some((i, rect)) if i < a.issues.len() && a.hover_key != Some(hover_key(&a.issues[i])) => {
            a.hover = Some(i);
            a.hover_key = Some(hover_key(&a.issues[i]));
            overlay::get().set_hover(Some(i));
            a.hover_left = None;
            let kind = a.issues[i].kind;
            let mut items = vec![a.issues[i].clone()];
            let para = a.issues[i].para.clone();
            items.extend(a.list.iter().filter(|e| e.kind == kind && e.issue.is_none() && e.para == para && e.fixed_para.is_some()).cloned());
            p.items = items;
            p.compact = settings().inline;
            p.note = None;
            p.selected = 0;
            p.busy = false;
            p.owner = overlay::get().owner;
            p.show_at(rect);
        }
        Some(_) => a.hover_left = None,
        None if a.hover.is_some() && !over_card => {
            let left = *a.hover_left.get_or_insert_with(std::time::Instant::now);
            if left.elapsed() > std::time::Duration::from_millis(350) {
                a.hover = None;
                a.hover_key = None;
                a.hover_left = None;
                overlay::get().set_hover(None);
                p.hide();
                sync_tab();
            }
        }
        None => a.hover_left = None,
    }
}

fn stop_pass_timers(hwnd: HWND) {
    unsafe {
        for t in [TIMER_DEBOUNCE, TIMER_SETTLE, TIMER_IDLE] {
            let _ = KillTimer(Some(hwnd), t);
        }
    }
}

/// Anchor for messages when the field itself couldn't be read: near the mouse-free
/// center of the foreground window.
fn fallback_anchor() -> Option<(HWND, RECT)> {
    unsafe {
        let h = GetForegroundWindow();
        let mut r = RECT::default();
        GetWindowRect(h, &mut r).ok()?;
        let cx = (r.left + r.right) / 2;
        let cy = (r.top + r.bottom) / 2;
        Some((h, RECT { left: cx - 120, top: cy, right: cx - 119, bottom: cy + 20 }))
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        #[allow(static_mut_refs)]
        if APP.is_none() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        match msg {
            WM_APP_TYPING => {
                on_typing(wp.0);
                LRESULT(0)
            }
            WM_APP_ACTION => {
                on_action(wp.0);
                LRESULT(0)
            }
            engine::WM_APP_RESULT => {
                on_results();
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_DEBOUNCE || wp.0 == TIMER_SETTLE || wp.0 == TIMER_IDLE => {
                let _ = KillTimer(Some(hwnd), wp.0);
                let a = app();
                // Don't start a background pass while the user is choosing in an on-demand popup.
                if a.on_demand {
                    return LRESULT(0);
                }
                let pass = match wp.0 {
                    TIMER_DEBOUNCE => Pass::Quick,
                    TIMER_SETTLE => Pass::Settle,
                    _ => Pass::Idle,
                };
                a.round = a.engine.request(pass);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_HOVER => {
                on_hover();
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_REMEASURE => {
                // Follow scrolling and window moves; drop the lines if the text changed.
                let a = app();
                if a.hover.is_none() && (!a.issues.is_empty() || a.ghost.is_some()) {
                    update_underlines(a);
                }
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_WATCH => {
                if overlay::get().visible() && GetForegroundWindow() != overlay::get().owner {
                    clear_underlines(app());
                }
                let p = popup::get();
                if p.visible() && GetForegroundWindow() != p.owner {
                    p.hide();
                }
                tray::set_tip(hwnd, &tooltip(&settings()));
                LRESULT(0)
            }
            tray::WM_APP_TRAY => {
                let event = (lp.0 & 0xFFFF) as u32;
                if event == WM_RBUTTONUP || event == WM_CONTEXTMENU || event == WM_LBUTTONUP {
                    let s = settings();
                    match tray::menu(hwnd, s.enabled) {
                        tray::CMD_TOGGLE => {
                            let mut s = s;
                            s.enabled = !s.enabled;
                            let _ = config::save(&s);
                            apply_settings_to_hook(&s);
                            app().engine.reconfigure(s);
                            popup::get().hide();
                        }
                        tray::CMD_SETTINGS => settings_ui::open(app().inst, hwnd, s),
                        tray::CMD_QUIT => {
                            let _ = DestroyWindow(hwnd);
                        }
                        _ => {}
                    }
                }
                LRESULT(0)
            }
            settings_ui::WM_APP_SETTINGS_SAVED => {
                let s = config::load();
                apply_settings_to_hook(&s);
                app().engine.reconfigure(s);
                LRESULT(0)
            }
            WM_DESTROY => {
                tray::remove(hwnd);
                PostQuitMessage(0);
                LRESULT(0)
            }
            m if m == app().taskbar_created => {
                tray::add(hwnd, &tooltip(&settings()));
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// Compatibility probe: `margin.exe --probe out.txt` logs what Margin can see in
/// the focused field once a second for 30 s. Used to verify app support honestly.
fn probe(out: String) {
    use std::io::Write;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let reader = uia::Reader::new().expect("UI Automation");
    let mut f = std::fs::File::create(&out).unwrap();
    for _ in 0..30 {
        let line = match reader.read_focused() {
            Ok(c) => {
                let tail: String = c.before.chars().rev().take(60).collect::<Vec<_>>().into_iter().rev().collect();
                format!(
                    "OK  exe={} via={} caret=({},{},{},{}) at_end={} sentence={:?} tail={:?}",
                    c.exe, c.source, c.caret.left, c.caret.top, c.caret.right, c.caret.bottom,
                    c.after_on_line.trim().is_empty(), text::current_sentence(&c.before).text, tail
                )
            }
            Err(e) => {
                let h = unsafe { GetForegroundWindow() };
                let mut pid = 0;
                unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
                format!("SKIP exe={} reason={e:?}", uia::process_name(pid))
            }
        };
        let _ = writeln!(f, "{line}");
        let _ = f.flush();
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
