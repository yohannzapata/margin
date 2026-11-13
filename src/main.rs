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
    /// When the mouse left both the underline and its card.
    hover_left: Option<std::time::Instant>,
    /// Predicted next words, shown as grey text after the caret (inline style).
    ghost: Option<engine::Suggestion>,
    /// The user asked for the full list (shortcut): show the popup even in inline style.
    show_list: bool,
    /// The current round only redoes Rephrase in a new style (other rows stay).
    restyle: bool,
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
            hover_left: None,
            ghost: None,
            show_list: false,
            restyle: false,
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
            let Some(item) = from_popup.or_else(|| a.ghost.clone()) else { return };
            p.hide();
            a.engine.cancel();
            stop_pass_timers(a.hwnd);
            clear_underlines(a);
            let caret_unmoved = a.reader.as_ref().map(|r| match r.read_focused() {
                Ok(ctx) => text::composition(&ctx.before, &ctx.after).target == item.basis,
                Err(_) => false,
            });
            match (caret_unmoved, &item.issue) {
                (Some(true) | None, _) => {
                    inject::apply(&item.plan);
                }
                // The caret moved (e.g. the user clicked the underlined word): select exactly
                // that word, verified to still read the same, and type the fix over it.
                (Some(false), Some(h)) => {
                    let ok = a.reader.as_ref().is_some_and(|r| r.select_span(&item.para, h.start, h.end, &h.orig));
                    if !ok {
                        return;
                    }
                    inject::replace_selection(&h.repl);
                }
                // The text changed since the suggestion was made: applying it could corrupt it.
                (Some(false), None) => return,
            }
            if item.issue.is_some() {
                // Re-check the rest of the paragraph (the earlier correction is reused, no model call).
                unsafe {
                    SetTimer(Some(a.hwnd), TIMER_DEBOUNCE, 150, None);
                    SetTimer(Some(a.hwnd), TIMER_SETTLE, 400, None);
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
    for u in updates {
        if u.id != a.round {
            continue; // superseded
        }
        if u.hide {
            p.hide();
            a.list.clear();
            clear_underlines(a);
            if let (Some(note), Some(anchor)) = (u.note, fallback_anchor()) {
                p.note = Some(note);
                p.owner = anchor.0;
                p.show_at(anchor.1);
            }
            continue;
        }
        let list = &mut a.list;
        if let Some((h, caret)) = u.anchor {
            let anchor = (HWND(h as _), caret);
            a.anchor = Some(anchor);
            if u.refresh {
                // Predictions only come from the quick pass; keep them. A restyle only
                // replaces Rephrase rows.
                let restyle = a.restyle;
                for it in list.iter_mut().filter(|i| i.kind != Kind::Next && (!restyle || i.kind == Kind::Rephrase)) {
                    it.stale = true;
                }
            } else {
                list.clear();
                p.selected = 0;
            }
            p.owner = anchor.0;
            p.note = a.on_demand.then(|| "Checking…".to_string());
        }
        for it in u.items {
            if it.from_model {
                // The model saw the whole message: it replaces the rule checker's rows and
                // any rows of its kind left from an earlier pass.
                list.retain(|e| !(e.kind == it.kind && (e.stale || !e.from_model)));
            } else if list.iter().any(|e| e.kind == it.kind && e.from_model) {
                continue; // a model row (even one awaiting refresh) beats the rule row
            } else {
                list.retain(|e| !(e.stale && e.kind == it.kind));
            }
            let dup = list.iter().any(|e| e.plan == it.plan && e.issue == it.issue);
            if !dup && (!it.plan.insert.is_empty() || it.plan.select > 0) {
                list.push(it);
            }
        }
        if u.done {
            list.retain(|e| !e.stale);
        }
        // Stable order: Fix all, individual fixes in text order, Improve, Rephrase, Next.
        list.sort_by_key(|i| (i.kind, i.issue.as_ref().map(|h| h.start + 1).unwrap_or(0)));
        list.truncate(9);
        let issues: Vec<engine::Suggestion> = list.iter().filter(|i| i.issue.is_some()).cloned().collect();
        let empty = list.is_empty();
        let inline = settings().inline && !a.show_list;
        a.ghost = if inline { a.list.iter().find(|i| i.kind == Kind::Next).cloned() } else { None };
        if u.done {
            a.on_demand = false;
        }
        // A hover card the user is looking at is never replaced by a late result.
        if a.hover.is_none() && inline {
            if p.visible() {
                p.hide();
            }
        } else if a.hover.is_none() {
            p.compact = false;
            // Style buttons whenever the list offers (or can offer) Rephrase.
            p.styles = (a.show_list && settings().rewriting).then(|| settings().style);
            p.items = a.list.clone();
            p.selected = p.selected.min(p.items.len().saturating_sub(1));
            p.busy = !u.done;
            if !empty {
                p.note = None;
            } else if let Some(n) = u.note {
                p.note = Some(n);
            } else if u.done && p.note.is_some() {
                p.note = Some("Looks good. No suggestions.".into());
            }
            if let Some((_, caret)) = a.anchor {
                p.show_at(caret);
            }
        }
        update_underlines(a, issues);
    }
}

/// Underline every individual fix in the app's text field.
fn update_underlines(a: &mut App, issues: Vec<engine::Suggestion>) {
    let ghost = a.ghost.as_ref().zip(a.anchor).map(|(g, (_, caret))| (caret, g.plan.insert.clone()));
    if issues.is_empty() {
        let owner = a.anchor.map(|x| x.0).unwrap_or_default();
        overlay::get().hide();
        a.issues.clear();
        if ghost.is_some() {
            overlay::get().show(owner, vec![], ghost);
        }
        sync_tab();
        return;
    }
    let para = issues[0].para.clone();
    let spans: Vec<(usize, usize, String)> = issues
        .iter()
        .map(|i| {
            let h = i.issue.as_ref().unwrap();
            (h.start, h.end, h.orig.clone())
        })
        .collect();
    let measured = a.reader.as_ref().and_then(|r| r.span_rects(&para, &spans));
    engine::dlog(&format!(
        "underline {} issue(s) {:?}: rects {:?}",
        spans.len(),
        spans.iter().map(|s| s.2.as_str()).collect::<Vec<_>>(),
        measured.as_ref().map(|v| v.iter().map(|r| r.len()).collect::<Vec<_>>())
    ));
    let Some(rects) = measured else {
        clear_underlines(a);
        return;
    };
    let color = |i: usize| if issues[i].kind == Kind::Improve { overlay::BLUE } else { overlay::RED };
    let hits: Vec<(usize, RECT, u32)> = rects.into_iter().enumerate().flat_map(|(i, rs)| rs.into_iter().map(move |r| (i, r, color(i)))).collect();
    let owner = a.anchor.map(|x| x.0).unwrap_or_default();
    a.issues = issues;
    overlay::get().show(owner, hits, ghost);
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
    match overlay::get().hit(pt) {
        Some((i, rect)) if a.hover != Some(i) && i < a.issues.len() => {
            a.hover = Some(i);
            overlay::get().set_hover(Some(i));
            a.hover_left = None;
            let kind = a.issues[i].kind;
            let mut items = vec![a.issues[i].clone()];
            items.extend(a.list.iter().filter(|e| e.kind == kind && e.issue.is_none()).cloned());
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
                    let issues = a.issues.clone();
                    update_underlines(a, issues);
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
