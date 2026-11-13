//! Compact native settings window (standard Win32 controls, common-controls v6).

use crate::config::{InferenceMode, Settings};
use crate::inference::Mode;
use crate::runtime::MODELS;
use std::sync::Mutex;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

pub const WM_APP_SETTINGS_SAVED: u32 = WM_APP + 5;
const CLASS: PCWSTR = w!("MarginSettings");

const ID_ENABLED: i32 = 101;
const ID_PREDICT: i32 = 102;
const ID_GRAMMAR: i32 = 103;
const ID_REWRITE: i32 = 104;
const ID_LANG: i32 = 105;
const ID_SHORTCUT: i32 = 106;
const ID_MODE: i32 = 107;
const ID_GPU: i32 = 108;
const ID_URL: i32 = 109;
const ID_MODEL: i32 = 110;
const ID_KEY: i32 = 111;
const ID_STATUS: i32 = 112;
const ID_EXCL: i32 = 113;
const ID_SAVE: i32 = 114;
const ID_CANCEL: i32 = 115;
const ID_WMODE: i32 = 116;
const ID_UNDERLINE: i32 = 117;
const ID_STYLE: i32 = 118;

const WRITING_MODES: &[(Mode, &str)] = &[
    (Mode::Correct, "Correct: fix mistakes, keep my wording"),
    (Mode::Improve, "Improve: clearer and smoother"),
    (Mode::Rephrase, "Rephrase: other ways to say it"),
];

const LANGS: &[(&str, &str)] = &[
    ("en-US", "English (United States)"),
    ("en-GB", "English (United Kingdom)"),
    ("en-CA", "English (Canada)"),
    ("en-AU", "English (Australia)"),
    ("en-IN", "English (India)"),
];

struct State {
    hwnd: isize,
    settings: Settings,
    owner: isize,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// The open settings window, for keyboard (Tab) navigation in the message loop.
pub fn hwnd() -> Option<HWND> {
    STATE.lock().unwrap().as_ref().filter(|s| s.hwnd != 0).map(|s| HWND(s.hwnd as _))
}

pub fn register(inst: HINSTANCE) {
    unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: inst,
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as isize as _),
            hIcon: crate::tray::icon(),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
}

pub fn open(inst: HINSTANCE, owner: HWND, settings: Settings) {
    if let Some(s) = STATE.lock().unwrap().as_ref() {
        unsafe {
            let h = HWND(s.hwnd as _);
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
        return;
    }
    *STATE.lock().unwrap() = Some(State { hwnd: 0, settings, owner: owner.0 as isize });
    unsafe {
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!("Margin settings"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT, CW_USEDEFAULT, 10, 10,
            None, None, Some(inst), None,
        )
        .unwrap();
        STATE.lock().unwrap().as_mut().unwrap().hwnd = hwnd.0 as isize;
        build(hwnd, inst);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        SetTimer(Some(hwnd), 1, 700, None);
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn build(hwnd: HWND, inst: HINSTANCE) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd).max(96) as i32;
        let sc = |v: i32| v * dpi / 96;
        let font = crate::theme::fonts(dpi);
        let s = STATE.lock().unwrap().as_ref().unwrap().settings.clone();
        let (w, m) = (sc(480), sc(20));
        let inner = w - 2 * m;
        let mut y = sc(16);

        let mk = |class: PCWSTR, text: &str, style: u32, x: i32, y: i32, cw: i32, ch: i32, id: i32| -> HWND {
            let t = wide(text);
            let h = CreateWindowExW(
                if class == w!("EDIT") { WS_EX_CLIENTEDGE } else { WINDOW_EX_STYLE(0) },
                class,
                PCWSTR(t.as_ptr()),
                WINDOW_STYLE(style) | WS_CHILD | WS_VISIBLE,
                x, y, cw, ch,
                Some(hwnd),
                Some(HMENU(id as isize as _)),
                Some(inst),
                None,
            )
            .unwrap();
            SendMessageW(h, WM_SETFONT, Some(WPARAM(font.body.0 as usize)), Some(LPARAM(1)));
            h
        };
        let heading = |text: &str, y: i32| {
            let h = mk(w!("STATIC"), text, 0, m, y, inner, sc(22), -1);
            SendMessageW(h, WM_SETFONT, Some(WPARAM(font.bold.0 as usize)), Some(LPARAM(1)));
        };
        let check = |text: &str, id: i32, on: bool, y: i32| {
            let h = mk(w!("BUTTON"), text, BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0, m, y, inner, sc(24), id);
            SendMessageW(h, BM_SETCHECK, Some(WPARAM(on as usize)), None);
        };

        heading("Writing assistance", y);
        y += sc(28);
        check("Turn on Margin", ID_ENABLED, s.enabled, y);
        y += sc(28);
        check("Predict the next words as I type", ID_PREDICT, s.predictive, y);
        y += sc(26);
        check("Correct spelling, grammar and punctuation", ID_GRAMMAR, s.grammar, y);
        y += sc(26);
        check("Show suggestions on the text instead of a popup", ID_UNDERLINE, s.inline, y);
        y += sc(26);
        check("Suggest clearer, more natural phrasing", ID_REWRITE, s.rewriting, y);
        y += sc(36);

        mk(w!("STATIC"), "While I type", 0, m, y + sc(4), sc(150), sc(22), -1);
        let wm = mk(w!("COMBOBOX"), "", (CBS_DROPDOWNLIST | WS_TABSTOP.0 as i32) as u32, m + sc(150), y, inner - sc(150), sc(160), ID_WMODE);
        for (i, (mode, name)) in WRITING_MODES.iter().enumerate() {
            let t = wide(name);
            SendMessageW(wm, CB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
            if *mode == s.writing_mode {
                SendMessageW(wm, CB_SETCURSEL, Some(WPARAM(i)), None);
            }
        }
        y += sc(34);

        mk(w!("STATIC"), "Rephrase style", 0, m, y + sc(4), sc(150), sc(22), -1);
        let st = mk(w!("COMBOBOX"), "", (CBS_DROPDOWNLIST | WS_TABSTOP.0 as i32) as u32, m + sc(150), y, inner - sc(150), sc(180), ID_STYLE);
        for (i, style) in crate::inference::Style::ALL.iter().enumerate() {
            let t = wide(style.label());
            SendMessageW(st, CB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
            if *style == s.style {
                SendMessageW(st, CB_SETCURSEL, Some(WPARAM(i)), None);
            }
        }
        y += sc(34);

        let label_w = sc(150);
        mk(w!("STATIC"), "Language", 0, m, y + sc(4), label_w, sc(22), -1);
        let lang = mk(w!("COMBOBOX"), "", (CBS_DROPDOWNLIST | WS_VSCROLL.0 as i32 | WS_TABSTOP.0 as i32) as u32, m + label_w, y, inner - label_w, sc(200), ID_LANG);
        for (i, (code, name)) in LANGS.iter().enumerate() {
            let t = wide(name);
            SendMessageW(lang, CB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
            if *code == s.language {
                SendMessageW(lang, CB_SETCURSEL, Some(WPARAM(i)), None);
            }
        }
        if SendMessageW(lang, CB_GETCURSEL, None, None).0 < 0 {
            SendMessageW(lang, CB_SETCURSEL, Some(WPARAM(0)), None);
        }
        y += sc(34);
        mk(w!("STATIC"), "Check now (all modes)", 0, m, y + sc(4), label_w, sc(22), -1);
        mk(w!("EDIT"), &s.on_demand_shortcut, (ES_AUTOHSCROLL as u32) | WS_TABSTOP.0, m + label_w, y, sc(180), sc(26), ID_SHORTCUT);
        y += sc(30);
        let hint = mk(w!("STATIC"), "While a suggestion is showing: Tab accepts, Alt+↑/↓ chooses, Esc dismisses.", 0, m, y, inner, sc(20), -1);
        SendMessageW(hint, WM_SETFONT, Some(WPARAM(font.small.0 as usize)), Some(LPARAM(1)));
        y += sc(34);

        heading("AI and privacy", y);
        y += sc(28);
        mk(w!("STATIC"), "Writing model", 0, m, y + sc(4), label_w, sc(22), -1);
        let mode = mk(w!("COMBOBOX"), "", (CBS_DROPDOWNLIST | WS_TABSTOP.0 as i32) as u32, m + label_w, y, inner - label_w, sc(200), ID_MODE);
        let mut entries: Vec<String> = vec![format!("On this PC: automatic ({})", crate::runtime::recommended(s.use_gpu).name)];
        entries.extend(MODELS.iter().map(|mi| format!("On this PC: {}", mi.name)));
        entries.push("My own server (text is sent to it)".into());
        entries.push("Off: grammar rules only".into());
        for e in &entries {
            let t = wide(e);
            SendMessageW(mode, CB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
        }
        let sel = match s.inference {
            InferenceMode::Local => MODELS.iter().position(|mi| mi.id == s.local_model).map(|i| i + 1).unwrap_or(0),
            InferenceMode::Remote => MODELS.len() + 1,
            InferenceMode::Off => MODELS.len() + 2,
        };
        SendMessageW(mode, CB_SETCURSEL, Some(WPARAM(sel)), None);
        y += sc(32);
        check("Use the graphics card when available", ID_GPU, s.use_gpu, y);
        y += sc(30);
        mk(w!("STATIC"), "Server URL", 0, m, y + sc(4), label_w, sc(22), -1);
        mk(w!("EDIT"), &s.remote_url, ES_AUTOHSCROLL as u32 | WS_TABSTOP.0, m + label_w, y, inner - label_w, sc(26), ID_URL);
        y += sc(30);
        mk(w!("STATIC"), "Model name", 0, m, y + sc(4), label_w, sc(22), -1);
        mk(w!("EDIT"), &s.remote_model, ES_AUTOHSCROLL as u32 | WS_TABSTOP.0, m + label_w, y, inner - label_w, sc(26), ID_MODEL);
        y += sc(30);
        mk(w!("STATIC"), "API key", 0, m, y + sc(4), label_w, sc(22), -1);
        mk(w!("EDIT"), &s.remote_key, (ES_AUTOHSCROLL | ES_PASSWORD) as u32 | WS_TABSTOP.0, m + label_w, y, inner - label_w, sc(26), ID_KEY);
        y += sc(34);
        mk(w!("STATIC"), &crate::runtime::status(), 0, m, y, inner, sc(20), ID_STATUS);
        y += sc(24);
        let note = mk(
            w!("STATIC"),
            "Margin never reads password fields, never saves what you type and sends no telemetry. \
             With an on-PC model nothing leaves this computer. With your own server, the sentence \
             you are writing is sent to that server.",
            0, m, y, inner, sc(54), -1,
        );
        SendMessageW(note, WM_SETFONT, Some(WPARAM(font.small.0 as usize)), Some(LPARAM(1)));
        y += sc(64);

        heading("Don't assist in", y);
        y += sc(24);
        let h = mk(w!("STATIC"), "One per line: an app (e.g. keepass.exe) or words in a window title (e.g. a website name).", 0, m, y, inner, sc(36), -1);
        SendMessageW(h, WM_SETFONT, Some(WPARAM(font.small.0 as usize)), Some(LPARAM(1)));
        y += sc(38);
        mk(
            w!("EDIT"),
            &s.exclusions.join("\r\n"),
            (ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32 | WS_VSCROLL.0 | WS_TABSTOP.0,
            m, y, inner, sc(96), ID_EXCL,
        );
        y += sc(110);

        let bw = sc(96);
        mk(w!("BUTTON"), "Save", BS_DEFPUSHBUTTON as u32 | WS_TABSTOP.0, w - m - 2 * bw - sc(8), y, bw, sc(30), ID_SAVE);
        mk(w!("BUTTON"), "Cancel", BS_PUSHBUTTON as u32 | WS_TABSTOP.0, w - m - bw, y, bw, sc(30), ID_CANCEL);
        y += sc(30) + m;

        sync_enabled(hwnd);
        let mut rc = RECT { left: 0, top: 0, right: w, bottom: y };
        let _ = AdjustWindowRectEx(&mut rc, WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU, false, WINDOW_EX_STYLE(0));
        let _ = SetWindowPos(hwnd, None, 0, 0, rc.right - rc.left, rc.bottom - rc.top, SWP_NOMOVE | SWP_NOZORDER);
    }
}

unsafe fn item(hwnd: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(hwnd), id).unwrap_or_default() }
}

unsafe fn checked(hwnd: HWND, id: i32) -> bool {
    unsafe { SendMessageW(item(hwnd, id), BM_GETCHECK, None, None).0 == 1 }
}

unsafe fn text_of(hwnd: HWND, id: i32) -> String {
    unsafe {
        let h = item(hwnd, id);
        let n = GetWindowTextLengthW(h);
        let mut buf = vec![0u16; n as usize + 1];
        let got = GetWindowTextW(h, &mut buf);
        String::from_utf16_lossy(&buf[..got as usize])
    }
}

unsafe fn sync_enabled(hwnd: HWND) {
    unsafe {
        let sel = SendMessageW(item(hwnd, ID_MODE), CB_GETCURSEL, None, None).0 as usize;
        let remote = sel == MODELS.len() + 1;
        let local = sel <= MODELS.len();
        for id in [ID_URL, ID_MODEL, ID_KEY] {
            let _ = EnableWindow(item(hwnd, id), remote);
        }
        let _ = EnableWindow(item(hwnd, ID_GPU), local);
    }
}

unsafe fn save(hwnd: HWND) -> std::result::Result<(), String> {
    unsafe {
        let mut st = STATE.lock().unwrap();
        let state = st.as_mut().unwrap();
        let mut s = state.settings.clone();
        s.enabled = checked(hwnd, ID_ENABLED);
        s.predictive = checked(hwnd, ID_PREDICT);
        s.grammar = checked(hwnd, ID_GRAMMAR);
        s.inline = checked(hwnd, ID_UNDERLINE);
        let si = SendMessageW(item(hwnd, ID_STYLE), CB_GETCURSEL, None, None).0.max(0) as usize;
        s.style = crate::inference::Style::ALL.get(si).copied().unwrap_or_default();
        s.rewriting = checked(hwnd, ID_REWRITE);
        let wi = SendMessageW(item(hwnd, ID_WMODE), CB_GETCURSEL, None, None).0.max(0) as usize;
        s.writing_mode = WRITING_MODES.get(wi).map(|w| w.0).unwrap_or(Mode::Correct);
        s.use_gpu = checked(hwnd, ID_GPU);
        let li = SendMessageW(item(hwnd, ID_LANG), CB_GETCURSEL, None, None).0.max(0) as usize;
        s.language = LANGS.get(li).map(|l| l.0).unwrap_or("en-US").into();
        let shortcut = text_of(hwnd, ID_SHORTCUT);
        if crate::hook::parse_shortcut(&shortcut).is_none() {
            return Err(format!("\"{shortcut}\" isn't a shortcut Margin understands. Try something like Ctrl+Shift+Space."));
        }
        s.on_demand_shortcut = shortcut;
        let sel = SendMessageW(item(hwnd, ID_MODE), CB_GETCURSEL, None, None).0.max(0) as usize;
        if sel <= MODELS.len() {
            s.inference = InferenceMode::Local;
            s.local_model = if sel == 0 { crate::runtime::AUTO.into() } else { MODELS[sel - 1].id.into() };
        } else if sel == MODELS.len() + 1 {
            s.inference = InferenceMode::Remote;
        } else {
            s.inference = InferenceMode::Off;
        }
        s.remote_url = text_of(hwnd, ID_URL).trim().into();
        s.remote_model = text_of(hwnd, ID_MODEL).trim().into();
        s.remote_key = text_of(hwnd, ID_KEY).trim().into();
        if s.inference == InferenceMode::Remote && !s.remote_url.starts_with("https://") {
            return Err("The server URL must start with https:// so your text is encrypted in transit.".into());
        }
        s.exclusions = text_of(hwnd, ID_EXCL).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        crate::config::save(&s).map_err(|e| format!("Couldn't save settings: {e}"))?;
        state.settings = s;
        let _ = PostMessageW(Some(HWND(state.owner as _)), WM_APP_SETTINGS_SAVED, WPARAM(0), LPARAM(0));
        Ok(())
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                let id = (wp.0 & 0xFFFF) as i32;
                let code = ((wp.0 >> 16) & 0xFFFF) as u32;
                match id {
                    ID_MODE if code == CBN_SELCHANGE => sync_enabled(hwnd),
                    ID_SAVE => match save(hwnd) {
                        Ok(()) => {
                            let _ = DestroyWindow(hwnd);
                        }
                        Err(e) => {
                            let t = wide(&e);
                            MessageBoxW(Some(hwnd), PCWSTR(t.as_ptr()), w!("Margin"), MB_ICONWARNING | MB_OK);
                        }
                    },
                    ID_CANCEL => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_TIMER => {
                let t = wide(&crate::runtime::status());
                let _ = SetWindowTextW(item(hwnd, ID_STATUS), PCWSTR(t.as_ptr()));
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC => {
                let dc = HDC(wp.0 as _);
                SetBkMode(dc, TRANSPARENT);
                if lp.0 as isize == item(hwnd, ID_STATUS).0 as isize {
                    SetTextColor(dc, COLORREF(0x006E6E6E));
                }
                LRESULT(GetSysColorBrush(COLOR_WINDOW).0 as isize)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                *STATE.lock().unwrap() = None;
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
