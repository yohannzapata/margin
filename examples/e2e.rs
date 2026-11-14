//! End-to-end compatibility test on the real desktop.
//!
//! For each target it: focuses the field, types a sentence with mistakes,
//! reads it back through UI Automation, applies Harper's fix with the same
//! injection path the app uses, reads again and checks the result.
//!
//! Safety: it only types when the foreground window belongs to the target it
//! launched, and it only closes processes it started itself.
//!
//!   cargo run --release --example e2e            # Win32 Edit, RichEdit, Edge editors
//!   cargo run --release --example e2e -- edge    # just the browser cases

use margin::grammar::Grammar;
use margin::text::{self, EditPlan};
use margin::uia::{Reader, Skip};
use margin::inject;
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

const TYPED: &str = "I dont think thats a good idea because its ";
const EXPECTED: &str = "I don't think that's a good idea because its ";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--host") {
        return host(args.get(2).map(String::as_str).unwrap_or("edit"));
    }
    let only = args.get(1).cloned().unwrap_or_default();
    // Tests change settings; put the user's file back exactly as it was afterwards.
    let settings_path = margin::config::dir().join("settings.json");
    let saved_settings = std::fs::read(&settings_path).ok();
    unsafe {
        // Physical pixels, like the app and UI Automation (cursor moves land where intended).
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let reader = Reader::new().expect("UI Automation");
    let uia: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).unwrap() };
    let mut grammar = Grammar::new(harper_core::Dialect::American);
    let mut report: Vec<String> = Vec::new();

    if only.is_empty() || only == "win32" {
        for kind in ["edit", "richedit"] {
            let mut child = Command::new(std::env::current_exe().unwrap()).args(["--host", kind]).spawn().unwrap();
            let title = format!("Margin E2E host {kind}");
            let r = match wait_window(|t| t == title, Duration::from_secs(10)) {
                Some(h) => run_case(&reader, &mut grammar, h, None, &uia),
                None => Err("window did not appear".into()),
            };
            report.push(format!("{:<28} {}", format!("Win32 {kind}"), fmt(r)));
            let _ = child.kill();
        }
    }

    if only.is_empty() || only == "edge" {
        let page = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pages/editors.html");
        let url = format!("file:///{}", page.display().to_string().replace('\\', "/"));
        let profile = std::env::temp_dir().join("margin-e2e-edge-profile");
        let edge = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe";
        let mut child: Child = Command::new(edge)
            .arg(format!("--user-data-dir={}", profile.display()))
            .args(["--no-first-run", "--no-default-browser-check", "--new-window", &url])
            .spawn()
            .expect("Edge");
        match wait_window(|t| t.starts_with("Margin editor test"), Duration::from_secs(20)) {
            Some(h) => {
                std::thread::sleep(Duration::from_millis(1500));
                for (label, name) in [("Edge textarea", "Textarea"), ("Edge contenteditable (Gmail-like)", "Message body"), ("Edge chat editor (Messenger-like)", "Chat message")] {
                    let r = run_case(&reader, &mut grammar, h, Some(name), &uia);
                    report.push(format!("{label:<34} {}", fmt(r)));
                }
                // Password fields must be refused.
                let r = focus_by_name(&uia, h, "Password").and_then(|_| {
                    std::thread::sleep(Duration::from_millis(300));
                    match reader.read_focused() {
                        Err(Skip::Password) => Ok("refused (password field)".to_string()),
                        Ok(_) => Err("READ A PASSWORD FIELD".to_string()),
                        Err(e) => Err(format!("unexpected: {e:?}")),
                    }
                });
                report.push(format!("{:<34} {}", "Edge password field", fmt(r)));
            }
            None => report.push("Edge: window did not appear".into()),
        }
        let _ = Command::new("taskkill").args(["/F", "/T", "/PID", &child.id().to_string()]).output();
        let _ = child.wait();
    }

    if only == "underline" {
        report.extend(underline_cases(&reader, &uia));
    }

    if only == "paragraphs" {
        set_inline(true);
        report.push(format!("{:<34} {}", "Paragraphs: RichEdit", fmt(paragraphs_case(&reader, &uia, None))));
        for (label, name) in [("Paragraphs: Edge textarea", "Textarea"), ("Paragraphs: Edge rich editor", "Message body")] {
            report.push(format!("{label:<34} {}", fmt(paragraphs_case(&reader, &uia, Some(name)))));
        }
    }

    if only == "styles" {
        set_inline(true);
        report.push(format!("{:<34} {}", "Styles: list → Formal → apply", fmt(style_case(&reader))));
    }

    if only == "latency" {
        set_inline(true);
        report.push(latency_case());
    }

    if only == "ghost" {
        set_inline(true);
        report.push(format!("{:<34} {}", "Inline: grey prediction → Tab", fmt(ghost_case(&reader))));
    }

    if only == "app" {
        set_inline(true);
        report.push(format!("{:<34} {}", "Inline: grey prediction → Tab", fmt(ghost_case(&reader))));
        // The popup list is still available as a setting; Tab applies its first row.
        set_inline(false);
        let ok = ["I don't think that's a good idea because its", "I don't think that's a good idea because it's"];
        report.push(format!("{:<34} {}", "Full app: type → popup → Tab", fmt(app_case(&reader, TYPED, 1200, &ok))));
        // The spec's example: one run-on message, then a pause long enough for the idle pass.
        let long = "hey i was thinking about what you said yesterday honestly i dont think thats going to work because we already tried something similar before and it didnt really go well";
        let want = [
            "Hey, I was thinking about what you said yesterday. Honestly, I don't think that's going to work because we already tried something similar before, and it didn't really go well.",
            "Hey, I was thinking about what you said yesterday. Honestly, I don't think that's going to work because we already tried something similar before and it didn't really go well.",
        ];
        report.push(format!("{:<34} {}", "Full app: whole message, idle", fmt(app_case(&reader, long, 4500, &want))));
        set_inline(true);
    }

    match saved_settings {
        Some(b) => {
            let _ = std::fs::write(&settings_path, b);
        }
        None => {
            let _ = std::fs::remove_file(&settings_path);
        }
    }
    println!("\n=== Margin E2E ===");
    for l in &report {
        println!("{l}");
    }
}

// ---------------------------------------------------------------------------
// Underlines + per-fix cards (QuillBot-style), driven with the real mouse.
// ---------------------------------------------------------------------------

fn window_text(h: HWND) -> String {
    margin::uia::window_title(h)
}

/// Underlined spans from the overlay's title: (issue index, rect).
fn underlines() -> Vec<(usize, RECT)> {
    let Ok(h) = (unsafe { FindWindowW(w!("MarginUnderlines"), None) }) else { return vec![] };
    if !unsafe { IsWindowVisible(h).as_bool() } {
        return vec![];
    }
    window_text(h)
        .split(';')
        .filter_map(|p| {
            let (i, r) = p.split_once(':')?;
            let v: Vec<i32> = r.split(',').filter_map(|x| x.parse().ok()).collect();
            (v.len() == 4).then(|| (i.parse().ok().unwrap_or(0), RECT { left: v[0], top: v[1], right: v[2], bottom: v[3] }))
        })
        .collect()
}

fn popup() -> Option<(HWND, RECT, String)> {
    let h = unsafe { FindWindowW(w!("MarginPopup"), None) }.ok()?;
    if !unsafe { IsWindowVisible(h).as_bool() } {
        return None;
    }
    let mut r = RECT::default();
    unsafe { GetWindowRect(h, &mut r).ok()? };
    Some((h, r, window_text(h)))
}

fn mouse_to(x: i32, y: i32) {
    unsafe {
        let _ = SetCursorPos(x, y);
    }
}

fn click() {
    let m = |f| INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: f, ..Default::default() } } };
    unsafe {
        SendInput(&[m(MOUSEEVENTF_LEFTDOWN), m(MOUSEEVENTF_LEFTUP)], size_of::<INPUT>() as i32);
    }
}

fn wait_for<T>(timeout_ms: u64, f: impl Fn() -> Option<T>) -> Option<T> {
    let t = Instant::now();
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if t.elapsed() > Duration::from_millis(timeout_ms) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Save a screen region as a 24-bit BMP (for checking how underlines and cards look).
fn capture(r: RECT, path: &std::path::Path) {
    use windows::Win32::Graphics::Gdi::*;
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    if w <= 0 || h <= 0 {
        return;
    }
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        let _ = BitBlt(mem, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY);
        let stride = ((w * 3 + 3) & !3) as usize;
        let mut buf = vec![0u8; stride * h as usize];
        let mut bi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: 40, biWidth: w, biHeight: h, biPlanes: 1, biBitCount: 24, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        GetDIBits(mem, bmp, 0, h as u32, Some(buf.as_mut_ptr() as _), &mut bi, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        let mut f = Vec::with_capacity(54 + buf.len());
        f.extend_from_slice(b"BM");
        f.extend_from_slice(&((54 + buf.len()) as u32).to_le_bytes());
        f.extend_from_slice(&[0; 4]);
        f.extend_from_slice(&54u32.to_le_bytes());
        f.extend_from_slice(&40u32.to_le_bytes());
        f.extend_from_slice(&w.to_le_bytes());
        f.extend_from_slice(&h.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&24u16.to_le_bytes());
        f.extend_from_slice(&[0; 24]);
        f.extend_from_slice(&buf);
        let _ = std::fs::write(path, f);
    }
}

fn center(r: &RECT) -> (i32, i32) {
    ((r.left + r.right) / 2, (r.top + r.bottom) / 2)
}

/// Click the first row of the visible card (rows start 4px below the top, 30px tall at 96 dpi).
fn click_first_row(card: &RECT, dpi: i32) {
    mouse_to(card.left + 14 * dpi / 96, card.top + 18 * dpi / 96);
    std::thread::sleep(Duration::from_millis(150));
    click();
}

fn one_underline_case(reader: &Reader, h: HWND, name: Option<&str>, uia: &IUIAutomation) -> std::result::Result<String, String> {
    const T: &str = "I dont think thats a good idea ";
    foreground(h)?;
    if let Some(n) = name {
        focus_by_name(uia, h, n)?;
    }
    std::thread::sleep(Duration::from_millis(300));
    if foreground_root() != h {
        return Err(focus_thief());
    }
    for u in T.encode_utf16() {
        let k = |f| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wScan: u, dwFlags: f, ..Default::default() } } };
        unsafe { SendInput(&[k(KEYEVENTF_UNICODE), k(KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32) };
        std::thread::sleep(Duration::from_millis(25));
    }
    let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(h) }.max(96) as i32;

    // 1. Both mistakes underlined.
    let t0 = Instant::now();
    let lines = wait_for(5000, || {
        let u = underlines();
        let mut ids: Vec<usize> = u.iter().map(|x| x.0).collect();
        ids.dedup();
        (ids.len() >= 2).then_some(u)
    })
    .ok_or_else(|| format!("expected 2 underlines, got {:?}", underlines()))?;
    let underline_ms = t0.elapsed().as_millis();
    // Issues are underlined in text order: dont (0), thats (1), maybe more.
    // Underlines are numbered by priority, not position: "thats" is the second from the left.
    let mut by_x: Vec<RECT> = lines.iter().map(|x| x.1).collect();
    by_x.sort_by_key(|r| r.left);
    let thats = by_x[1];

    // 2. Hover "thats" → card with only that fix → click it.
    let (x, y) = center(&thats);
    mouse_to(x, y);
    let (_, card, title) = wait_for(2000, || popup().filter(|p| p.2.starts_with("Spelling: thats")))
        .ok_or_else(|| format!("no card for 'thats' on hover (popup: {:?})", popup().map(|p| p.2)))?;
    if let Ok(dir) = std::env::var("MARGIN_SHOTS") {
        // Underlined text plus the hover card, as the user sees them.
        let left = lines.iter().map(|x| x.1.left).min().unwrap_or(card.left).min(card.left) - 20;
        let top = lines.iter().map(|x| x.1.top).min().unwrap_or(card.top) - 20;
        let shot = RECT { left, top, right: card.right.max(thats.right) + 20, bottom: card.bottom + 10 };
        std::thread::sleep(Duration::from_millis(250));
        capture(shot, &std::path::Path::new(&dir).join(format!("underline-{}.bmp", name.unwrap_or("richedit").replace(' ', "-"))));
    }
    click_first_row(&card, dpi);
    std::thread::sleep(Duration::from_millis(600));
    let after = retry(|| reader.read_focused())?;
    // The whole text is right, and the caret is back at the end where the user was typing.
    let whole = format!("{}{}", after.before, after.after);
    if whole.trim_end() != "I dont think that's a good idea" || !after.after.trim().is_empty() {
        return Err(format!("after fixing 'thats': text {whole:?}, text after caret {:?}", after.after));
    }

    // 3. Click on "dont" (moves the caret into the word), then use its card.
    let dont = wait_for(4000, || underlines().into_iter().map(|x| x.1).min_by_key(|r| r.left))
        .ok_or("underline for 'dont' did not come back after the first fix")?;
    let (x, y) = center(&dont);
    mouse_to(x, y);
    std::thread::sleep(Duration::from_millis(100));
    click();
    let (_, card2, _) = wait_for(2000, || popup().filter(|p| p.2.contains("dont")))
        .ok_or_else(|| format!("no card for 'dont' (popup: {:?})", popup().map(|p| p.2)))?;
    click_first_row(&card2, dpi);
    std::thread::sleep(Duration::from_millis(600));
    let field = retry(|| reader.read_focused())?;
    let all = format!("{}{}", field.before, field.after);
    if !all.contains("I don't think that's a good idea") || all.contains("dont") {
        return Err(format!("after fixing 'dont' with the caret inside it: {all:?}"));
    }
    if foreground_root() != h {
        return Err("focus left the app".into());
    }
    mouse_to(10, 10);
    Ok(format!("underlines after {underline_ms} ms; card \"{title}\"; single fix at end and with caret moved both applied"))
}

fn set_inline(on: bool) {
    let mut s = margin::config::load();
    s.inline = on;
    s.predictive = true; // the ghost test needs predictions
    let _ = margin::config::save(&s);
}

fn underline_cases(reader: &Reader, uia: &IUIAutomation) -> Vec<String> {
    let mut out = vec![];
    set_inline(true);
    let exe = std::path::Path::new(&std::env::var("LOCALAPPDATA").unwrap()).join("Margin/target/release/margin.exe");
    let mut app = Command::new(exe).spawn().expect("margin");
    std::thread::sleep(Duration::from_secs(8));

    let mut host = Command::new(std::env::current_exe().unwrap()).args(["--host", "richedit"]).spawn().unwrap();
    let r = match wait_window(|t| t == "Margin E2E host richedit", Duration::from_secs(10)) {
        Some(h) => one_underline_case(reader, h, None, uia),
        None => Err("host window missing".into()),
    };
    out.push(format!("{:<34} {}", "Underlines: Win32 RichEdit", fmt(r)));
    let _ = host.kill();

    let page = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pages/editors.html");
    let url = format!("file:///{}", page.display().to_string().replace('\\', "/"));
    let profile = std::env::temp_dir().join("margin-e2e-edge-profile");
    let mut edge = Command::new(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")
        .arg(format!("--user-data-dir={}", profile.display()))
        .args(["--no-first-run", "--no-default-browser-check", "--new-window", &url])
        .spawn()
        .expect("Edge");
    if let Some(h) = wait_window(|t| t.starts_with("Margin editor test"), Duration::from_secs(20)) {
        std::thread::sleep(Duration::from_millis(1500));
        for (label, name) in [("Underlines: Edge textarea", "Textarea"), ("Underlines: Edge chat editor", "Chat message")] {
            let r = one_underline_case(reader, h, Some(name), uia);
            out.push(format!("{label:<34} {}", fmt(r)));
        }
    } else {
        out.push("Edge: window did not appear".into());
    }
    let _ = Command::new("taskkill").args(["/F", "/T", "/PID", &edge.id().to_string()]).output();
    let _ = edge.wait();
    let _ = Command::new("taskkill").args(["/F", "/PID", &app.id().to_string()]).output();
    let _ = app.wait();
    out
}

/// Two paragraphs; the caret ends in the second. The first must get underlined and be
/// fixable from there, leaving the second untouched.
fn paragraphs_case(reader: &Reader, uia: &IUIAutomation, edge_field: Option<&str>) -> std::result::Result<String, String> {
    const P1: &str = "we went to the beach yesterday and i think its the best day of the summer";
    const P2: &str = "lets meet tomorow at the cafe ";
    let exe = std::path::Path::new(&std::env::var("LOCALAPPDATA").unwrap()).join("Margin/target/release/margin.exe");
    let mut app = Command::new(exe).spawn().map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_secs(10));
    let mut host: Child = match edge_field {
        None => Command::new(std::env::current_exe().unwrap()).args(["--host", "richedit"]).spawn().unwrap(),
        Some(_) => {
            let page = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pages/editors.html");
            let url = format!("file:///{}", page.display().to_string().replace('\\', "/"));
            let profile = std::env::temp_dir().join("margin-e2e-edge-profile");
            Command::new(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")
                .arg(format!("--user-data-dir={}", profile.display()))
                .args(["--no-first-run", "--no-default-browser-check", "--new-window", &url])
                .spawn()
                .map_err(|e| e.to_string())?
        }
    };
    let result = (|| {
        let h = match edge_field {
            None => wait_window(|t| t == "Margin E2E host richedit", Duration::from_secs(10)),
            Some(_) => wait_window(|t| t.starts_with("Margin editor test"), Duration::from_secs(20)),
        }
        .ok_or("window missing")?;
        if edge_field.is_some() {
            std::thread::sleep(Duration::from_millis(1500));
        }
        foreground(h)?;
        if let Some(n) = edge_field {
            focus_by_name(uia, h, n)?;
        }
        std::thread::sleep(Duration::from_millis(300));
        guarded_type(h, &margin::text::EditPlan { select: 0, insert: P1.into() })?;
        let enter = |up: bool| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_RETURN, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, ..Default::default() } } };
        if foreground_root() != h {
            return Err(focus_thief());
        }
        unsafe { SendInput(&[enter(false), enter(true)], size_of::<INPUT>() as i32) };
        std::thread::sleep(Duration::from_millis(100));
        guarded_type(h, &margin::text::EditPlan { select: 0, insert: P2.into() })?;
        let caret_y = retry(|| reader.read_focused())?.caret.top;
        // Underlines must appear above the caret line (in the first paragraph).
        let t0 = Instant::now();
        let first = wait_for(6000, || {
            let u = underlines();
            u.into_iter().filter(|x| x.1.bottom <= caret_y).min_by_key(|x| x.1.left)
        })
        .ok_or_else(|| format!("no underline in the first paragraph (underlines: {:?})", underlines()))?;
        let found_ms = t0.elapsed().as_millis();
        let n_first = underlines().iter().filter(|x| x.1.bottom <= caret_y).count();
        let (x, y) = center(&first.1);
        mouse_to(x, y);
        let (ph, card, title) = wait_for(2000, popup).ok_or("no card on hover")?;
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(ph) }.max(96) as i32;
        let before_fix = retry(|| reader.read_focused())?;
        let p1_before = before_fix.before.split(['\r', '\n']).next().unwrap_or("").to_string();
        click_first_row(&card, dpi);
        std::thread::sleep(Duration::from_millis(700));
        mouse_to(10, 10);
        let after = retry(|| reader.read_focused())?;
        let all = format!("{}{}", after.before, after.after);
        let mut lines = all.split(['\r', '\n']).filter(|l| !l.is_empty());
        let (p1, p2) = (lines.next().unwrap_or(""), lines.next().unwrap_or(""));
        if p1 == p1_before {
            return Err(format!("first paragraph unchanged after clicking the card {title:?}: {p1:?}"));
        }
        if p2.trim_end() != P2.trim_end() {
            return Err(format!("second paragraph was changed: {p2:?}"));
        }
        // The caret is back at the end of paragraph 2, where the user was typing.
        if !after.after.trim().is_empty() {
            return Err(format!("caret not restored: text after it {:?}", after.after));
        }
        Ok(format!("{n_first} underline(s) in paragraph 1 after {found_ms} ms; card {title:?}; paragraph 1 now {p1:?}; caret restored"))
    })();
    match edge_field {
        None => {
            let _ = host.kill();
        }
        Some(_) => {
            let _ = Command::new("taskkill").args(["/F", "/T", "/PID", &host.id().to_string()]).output();
        }
    }
    let _ = host.wait();
    let _ = Command::new("taskkill").args(["/F", "/PID", &app.id().to_string()]).output();
    let _ = app.wait();
    result
}

/// Rows of the visible popup, from its title ("Tag: text | Tag: text").
fn popup_rows() -> Vec<String> {
    popup().map(|p| p.2.split(" | ").map(str::to_string).collect()).unwrap_or_default()
}

/// Shortcut list → style buttons → Formal → apply a formal rewrite.
fn style_case(reader: &Reader) -> std::result::Result<String, String> {
    const T: &str = "gonna be late, traffic is crazy rn.";
    // Start from Standard so switching to Formal is a real change.
    let mut s = margin::config::load();
    s.style = margin::inference::Style::Standard;
    let _ = margin::config::save(&s);
    let exe = std::path::Path::new(&std::env::var("LOCALAPPDATA").unwrap()).join("Margin/target/release/margin.exe");
    let mut app = Command::new(exe).spawn().map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_secs(10));
    let mut host = Command::new(std::env::current_exe().unwrap()).args(["--host", "richedit"]).spawn().unwrap();
    let result = (|| {
        let h = wait_window(|t| t == "Margin E2E host richedit", Duration::from_secs(10)).ok_or("host window missing")?;
        foreground(h)?;
        std::thread::sleep(Duration::from_millis(300));
        if foreground_root() != h {
            return Err(focus_thief());
        }
        guarded_type(h, &margin::text::EditPlan { select: 0, insert: T.into() })?;
        std::thread::sleep(Duration::from_millis(400));
        // Ctrl+Shift+Space: the full list.
        let k = |vk: VIRTUAL_KEY, up: bool| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, ..Default::default() } } };
        unsafe {
            SendInput(&[k(VK_CONTROL, false), k(VK_SHIFT, false), k(VK_SPACE, false), k(VK_SPACE, true), k(VK_SHIFT, true), k(VK_CONTROL, true)], size_of::<INPUT>() as i32);
        }
        wait_for(3000, popup).ok_or("the list did not open")?;
        // Let the Standard pass finish. (Standard may rightly offer nothing for slangy text:
        // it must keep "gonna"/"rn", and rewrites that drop them are rejected.)
        std::thread::sleep(Duration::from_secs(5));
        let standard = popup_rows();
        let (ph, card, _) = popup().ok_or("popup gone")?;
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(ph) }.max(96) as i32;
        let sc = |v: i32| v * dpi / 96;
        // Formal is the second style button; widths depend on text, so walk along the strip
        // until a click saves Formal as the chosen style.
        let mut found = false;
        for x in (sc(20)..sc(220)).step_by(sc(10) as usize) {
            mouse_to(card.left + x, card.top + sc(18));
            click();
            std::thread::sleep(Duration::from_millis(150));
            if margin::config::load().style == margin::inference::Style::Formal {
                found = true;
                break;
            }
        }
        if !found {
            return Err(format!("clicking the style buttons never selected Formal (rows: {:?})", popup_rows()));
        }
        let restyled = wait_for(10000, || {
            let rows = popup_rows();
            rows.iter().any(|r| r.starts_with("Rephrase:")).then_some(rows)
        })
        .ok_or_else(|| format!("no rephrasings after switching style: {:?}", popup_rows()))?;
        let saved = margin::config::load().style;
        if let Ok(dir) = std::env::var("MARGIN_SHOTS") {
            std::thread::sleep(Duration::from_millis(250));
            if let Some((_, r, _)) = popup() {
                capture(RECT { left: r.left - 10, top: r.top - 10, right: r.right + 10, bottom: r.bottom + 10 }, &std::path::Path::new(&dir).join("styles.bmp"));
            }
        }
        // Click the first Rephrase row (rows start under the 36px style strip).
        let idx = restyled.iter().position(|r| r.starts_with("Rephrase:")).unwrap() as i32;
        let (_, card, _) = popup().ok_or("popup gone")?;
        mouse_to(card.left + sc(40), card.top + sc(36 + 4 + 15) + idx * sc(30));
        std::thread::sleep(Duration::from_millis(150));
        click();
        std::thread::sleep(Duration::from_millis(700));
        let after = retry(|| reader.read_focused())?;
        mouse_to(10, 10);
        let line = after.before.trim().to_string();
        if line == T || line.contains("gonna") {
            return Err(format!("text not replaced by the formal rewrite: {line:?}"));
        }
        let std_rows: Vec<&String> = standard.iter().filter(|r| r.starts_with("Rephrase:")).collect();
        let formal_rows: Vec<&String> = restyled.iter().filter(|r| r.starts_with("Rephrase:")).collect();
        Ok(format!("style saved as {saved:?}; standard {std_rows:?} → formal {formal_rows:?}; applied: {line:?}"))
    })();
    let _ = host.kill();
    let _ = Command::new("taskkill").args(["/F", "/PID", &app.id().to_string()]).output();
    let _ = app.wait();
    result
}

/// What the user waits for: timeline of underlines/ghost after the last keystroke,
/// plus whether underlines stay visible while typing. Types at ~8 chars/s like a person.
fn latency_case() -> String {
    // "its" (→ it's) is left to the model; Harper catches dont/thats instantly.
    const T: &str = "I dont think thats a good idea because its too";
    let exe = std::path::Path::new(&std::env::var("LOCALAPPDATA").unwrap()).join("Margin/target/release/margin.exe");
    let mut app = Command::new(exe).spawn().unwrap();
    std::thread::sleep(Duration::from_secs(10));
    let mut host = Command::new(std::env::current_exe().unwrap()).args(["--host", "richedit"]).spawn().unwrap();
    let out = (|| -> std::result::Result<String, String> {
        let h = wait_window(|t| t == "Margin E2E host richedit", Duration::from_secs(10)).ok_or("host window missing")?;
        foreground(h)?;
        std::thread::sleep(Duration::from_millis(300));
        if foreground_root() != h {
            return Err(focus_thief());
        }
        let state = || {
            let u = underlines();
            let mut ids: Vec<usize> = u.iter().map(|x| x.0).collect();
            ids.dedup();
            let ghost = unsafe { FindWindowW(w!("MarginUnderlines"), None) }.ok().map(window_text).is_some_and(|t| t.contains("ghost:"));
            (ids.len(), ghost)
        };
        // While typing: how often are underlines on screen (sampled before each key)?
        let (mut samples, mut shown_while_typing) = (0, 0);
        let mut first_underline_mid_typing = None;
        let t_type = Instant::now();
        for (i, u) in T.encode_utf16().enumerate() {
            if i > 12 {
                samples += 1;
                if state().0 > 0 {
                    shown_while_typing += 1;
                    first_underline_mid_typing.get_or_insert(t_type.elapsed().as_millis());
                }
            }
            let k = |f| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wScan: u, dwFlags: f, ..Default::default() } } };
            unsafe { SendInput(&[k(KEYEVENTF_UNICODE), k(KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32) };
            std::thread::sleep(Duration::from_millis(120));
        }
        // After the last key: timeline of changes for 3 s.
        let t0 = Instant::now();
        let mut last = (usize::MAX, false);
        let mut timeline = vec![];
        while t0.elapsed() < Duration::from_secs(3) {
            let s = state();
            if s != last {
                timeline.push(format!("{}ms: {} underlined{}", t0.elapsed().as_millis(), s.0, if s.1 { " + grey prediction" } else { "" }));
                last = s;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(format!(
            "while typing: underlines visible in {shown_while_typing}/{samples} samples; after last key: {}",
            timeline.join(", ")
        ))
    })();
    let _ = host.kill();
    let _ = Command::new("taskkill").args(["/F", "/PID", &app.id().to_string()]).output();
    let _ = app.wait();
    format!("{:<34} {}", "Latency (RichEdit)", match out { Ok(s) => s, Err(e) => format!("FAIL {e}") })
}

/// Inline style: grey predicted words appear after the caret; Tab types them.
fn ghost_case(reader: &Reader) -> std::result::Result<String, String> {
    const T: &str = "Thanks for getting back to me so";
    let exe = std::path::Path::new(&std::env::var("LOCALAPPDATA").unwrap()).join("Margin/target/release/margin.exe");
    let mut app = Command::new(exe).spawn().map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_secs(8));
    let mut host = Command::new(std::env::current_exe().unwrap()).args(["--host", "richedit"]).spawn().unwrap();
    let result = (|| {
        let h = wait_window(|t| t == "Margin E2E host richedit", Duration::from_secs(10)).ok_or("host window missing")?;
        foreground(h)?;
        std::thread::sleep(Duration::from_millis(300));
        if foreground_root() != h {
            return Err(focus_thief());
        }
        for u in T.encode_utf16() {
            let k = |f| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wScan: u, dwFlags: f, ..Default::default() } } };
            unsafe { SendInput(&[k(KEYEVENTF_UNICODE), k(KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32) };
            std::thread::sleep(Duration::from_millis(25));
        }
        let t0 = Instant::now();
        let ghost = wait_for(3000, || {
            let o = unsafe { FindWindowW(w!("MarginUnderlines"), None) }.ok()?;
            let t = window_text(o);
            let g = t.split(';').find(|p| p.starts_with("ghost:"))?.trim_start_matches("ghost:").to_string();
            let v: Vec<i32> = g.split(',').filter_map(|x| x.parse().ok()).collect();
            (v.len() == 4).then(|| RECT { left: v[0], top: v[1], right: v[2], bottom: v[3] })
        })
        .ok_or_else(|| {
            let o = unsafe { FindWindowW(w!("MarginUnderlines"), None) }.ok();
            format!(
                "no grey prediction appeared (overlay visible: {:?}, title: {:?}; popup: {:?})",
                o.map(|o| unsafe { IsWindowVisible(o).as_bool() }),
                o.map(window_text),
                popup().map(|p| p.2)
            )
        })?;
        let ghost_ms = t0.elapsed().as_millis();
        if popup().is_some() {
            return Err("a popup appeared in inline mode".into());
        }
        if let Ok(dir) = std::env::var("MARGIN_SHOTS") {
            std::thread::sleep(Duration::from_millis(200));
            let mut r = RECT::default();
            let _ = unsafe { GetWindowRect(h, &mut r) };
            capture(RECT { left: r.left, top: ghost.top - 30, right: ghost.right + 60, bottom: ghost.bottom + 30 }, &std::path::Path::new(&dir).join("ghost.bmp"));
        }
        let tab = |f| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_TAB, dwFlags: f, ..Default::default() } } };
        unsafe { SendInput(&[tab(KEYBD_EVENT_FLAGS(0)), tab(KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32) };
        std::thread::sleep(Duration::from_millis(500));
        let after = retry(|| reader.read_focused())?;
        let added = after.before.strip_prefix(T).unwrap_or("").to_string();
        if added.trim().is_empty() || added.contains('\t') {
            return Err(format!("Tab did not insert the prediction: {:?}", after.before));
        }
        Ok(format!("grey text after {ghost_ms} ms, no popup; Tab typed {added:?}"))
    })();
    let _ = host.kill();
    let _ = Command::new("taskkill").args(["/F", "/PID", &app.id().to_string()]).output();
    let _ = app.wait();
    result
}

/// The real margin.exe: type like a user, wait for the popup, press Tab like a user.
fn app_case(reader: &Reader, typed: &str, wait_ms: u64, want: &[&str]) -> std::result::Result<String, String> {
    // Skip the first-run settings window so it doesn't take focus.
    if !margin::config::dir().join("settings.json").exists() {
        margin::config::save(&margin::config::Settings::default()).map_err(|e| e.to_string())?;
    }
    let exe = std::path::Path::new(&std::env::var("LOCALAPPDATA").unwrap()).join("Margin/target/release/margin.exe");
    let mut app = Command::new(exe).spawn().map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_secs(6)); // model server start + warm-up
    let mut host = Command::new(std::env::current_exe().unwrap()).args(["--host", "edit"]).spawn().unwrap();
    let result = (|| {
        let h = wait_window(|t| t == "Margin E2E host edit", Duration::from_secs(10)).ok_or("host window missing")?;
        foreground(h)?;
        std::thread::sleep(Duration::from_millis(300));
        // Type character by character, like a person, without the injection marker.
        if foreground_root() != h {
            return Err(focus_thief());
        }
        for u in typed.encode_utf16() {
            let k = |f| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wScan: u, dwFlags: f, ..Default::default() } } };
            unsafe { SendInput(&[k(KEYEVENTF_UNICODE), k(KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32) };
            std::thread::sleep(Duration::from_millis(25));
        }
        let t = Instant::now();
        let popup = loop {
            let p = unsafe { FindWindowW(w!("MarginPopup"), None) }.ok().filter(|p| unsafe { IsWindowVisible(*p).as_bool() });
            if let Some(p) = p {
                break p;
            }
            if t.elapsed() > Duration::from_secs(4) {
                return Err("popup never appeared".to_string());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let shown_ms = t.elapsed().as_millis();
        std::thread::sleep(Duration::from_millis(wait_ms)); // let model passes (settle/idle) run
        let mut r = RECT::default();
        let _ = unsafe { GetWindowRect(popup, &mut r) };
        if foreground_root() != h {
            return Err("popup stole focus".to_string());
        }
        let tab = |f| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_TAB, dwFlags: f, ..Default::default() } } };
        unsafe { SendInput(&[tab(KEYBD_EVENT_FLAGS(0)), tab(KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32) };
        std::thread::sleep(Duration::from_millis(600));
        let after = retry(|| reader.read_focused())?;
        let ok = want.iter().any(|e| after.before.trim_end().ends_with(e));
        if after.before.contains('\t') {
            return Err("Tab leaked into the field".into());
        }
        if ok {
            Ok(format!("popup in {shown_ms} ms after last key, {}px tall, focus kept, Tab applied: {:?}", r.bottom - r.top, after.before.trim_end()))
        } else {
            Err(format!("after Tab: {:?}", after.before))
        }
    })();
    let _ = host.kill();
    let _ = Command::new("taskkill").args(["/F", "/PID", &app.id().to_string()]).output();
    let _ = app.wait();
    result
}

fn fmt(r:std::result::Result<String, String>) -> String {
    match r {
        Ok(s) => format!("PASS  {s}"),
        Err(e) => format!("FAIL  {e}"),
    }
}

fn run_case(reader: &Reader, grammar: &mut Grammar, hwnd: HWND, name: Option<&str>, uia: &IUIAutomation) -> std::result::Result<String, String> {
    foreground(hwnd)?;
    if let Some(n) = name {
        focus_by_name(uia, hwnd, n)?;
    }
    std::thread::sleep(Duration::from_millis(300));
    guarded_type(hwnd, &EditPlan { select: 0, insert: TYPED.into() })?;
    std::thread::sleep(Duration::from_millis(500));

    let t = Instant::now();
    let ctx = retry(|| reader.read_focused())?;
    if ctx.before.contains(EXPECTED.trim()) {
        return Err("read text from outside the focused field".into());
    }
    let read_ms = t.elapsed().as_millis();
    if !ctx.before.ends_with(TYPED) {
        return Err(format!("read {:?}, expected it to end with the typed text", ctx.before));
    }
    let sentence = text::current_sentence(&ctx.before);
    let t = Instant::now();
    let fixed = grammar.correct(&sentence.text, sentence.finished).ok_or("no fix proposed")?;
    let lint_ms = t.elapsed().as_millis();
    let plan = text::plan_edit(&sentence.text, &fixed);
    guarded_type(hwnd, &plan)?;
    std::thread::sleep(Duration::from_millis(500));
    let after = retry(|| reader.read_focused())?;
    let caret_ok = after.caret.bottom > after.caret.top && after.caret.left > 0;
    if after.before.ends_with(EXPECTED) {
        Ok(format!("via {}, read {read_ms} ms, lint {lint_ms} ms, caret {}", ctx.source, if caret_ok { "found" } else { "MISSING" }))
    } else {
        Err(format!("after fix read {:?}", after.before))
    }
}

fn retry(f: impl Fn() -> std::result::Result<margin::uia::FieldContext, Skip>) -> std::result::Result<margin::uia::FieldContext, String> {
    let mut last = String::new();
    for _ in 0..5 {
        match f() {
            Ok(c) => return Ok(c),
            Err(e) => last = format!("{e:?}"),
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Err(format!("could not read field: {last}"))
}

/// Type only if the target is still the foreground window.
/// The top-level window that has the foreground. (Edge sometimes reports its inner
/// "Chrome Legacy Window" child as the foreground window; that is still Edge.)
fn foreground_root() -> HWND {
    unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) }
}

/// Which window took the foreground (for diagnosing focus failures).
fn focus_thief() -> String {
    let h = unsafe { GetForegroundWindow() };
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    format!("target lost focus to {:?} ({}); refusing to type", window_text(h), margin::uia::process_name(pid))
}

fn guarded_type(hwnd: HWND, plan: &EditPlan) -> std::result::Result<(), String> {
    if foreground_root() != hwnd {
        return Err(focus_thief());
    }
    if inject::apply(plan) { Ok(()) } else { Err("SendInput was blocked".into()) }
}

fn foreground(hwnd: HWND) -> std::result::Result<(), String> {
    unsafe {
        for _ in 0..10 {
            // A harmless key event lets this process change the foreground window.
            let k = |f| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0xE8), dwFlags: f, dwExtraInfo: margin::hook::INJECT_MAGIC, ..Default::default() } } };
            SendInput(&[k(KEYBD_EVENT_FLAGS(0)), k(KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
            std::thread::sleep(Duration::from_millis(200));
            if GetForegroundWindow() == hwnd {
                return Ok(());
            }
        }
    }
    Err("could not bring the test window to the front".into())
}

fn focus_by_name(uia: &IUIAutomation, hwnd: HWND, name: &str) -> std::result::Result<(), String> {
    unsafe {
        let root = uia.ElementFromHandle(hwnd).map_err(|e| e.to_string())?;
        let cond = uia.CreatePropertyCondition(UIA_NamePropertyId, &windows::Win32::System::Variant::VARIANT::from(name)).map_err(|e| e.to_string())?;
        for _ in 0..20 {
            if let Ok(el) = root.FindFirst(TreeScope_Descendants, &cond) {
                el.SetFocus().map_err(|e| e.to_string())?;
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Err(format!("field {name:?} not found in accessibility tree"))
    }
}

fn wait_window(pred: impl Fn(&str) -> bool, timeout: Duration) -> Option<HWND> {
    let t = Instant::now();
    while t.elapsed() < timeout {
        let mut found: Vec<(HWND, String)> = Vec::new();
        unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> BOOL {
            unsafe {
                let v = &mut *(lp.0 as *mut Vec<(HWND, String)>);
                if IsWindowVisible(h).as_bool() {
                    v.push((h, margin::uia::window_title(h)));
                }
                TRUE
            }
        }
        unsafe {
            let _ = EnumWindows(Some(cb), LPARAM(&mut found as *mut _ as isize));
        }
        if let Some((h, _)) = found.into_iter().find(|(_, t)| pred(t)) {
            return Some(h);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    None
}

/// Child process: a top-level window with a classic EDIT or RichEdit control.
fn host(kind: &str) {
    unsafe {
        let inst: HINSTANCE = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap().into();
        let class = if kind == "richedit" {
            let _ = windows::Win32::System::LibraryLoader::LoadLibraryW(w!("Msftedit.dll"));
            w!("RICHEDIT50W")
        } else {
            w!("EDIT")
        };
        let wc = WNDCLASSW { lpfnWndProc: Some(host_proc), hInstance: inst, lpszClassName: w!("MarginE2EHost"), ..Default::default() };
        RegisterClassW(&wc);
        let title: Vec<u16> = format!("Margin E2E host {kind}").encode_utf16().chain([0]).collect();
        let top = CreateWindowExW(WINDOW_EX_STYLE(0), w!("MarginE2EHost"), PCWSTR(title.as_ptr()), WS_OVERLAPPEDWINDOW | WS_VISIBLE, 200, 200, 640, 240, None, None, Some(inst), None).unwrap();
        let edit = CreateWindowExW(WS_EX_CLIENTEDGE, class, w!(""), WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0004 | 0x0040), 10, 10, 600, 180, Some(top), None, Some(inst), None).unwrap();
        let _ = SetFocus(Some(edit));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn host_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if m == WM_DESTROY {
            PostQuitMessage(0);
            return LRESULT(0);
        }
        DefWindowProcW(h, m, w, l)
    }
}