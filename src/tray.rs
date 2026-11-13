//! Notification-area icon and menu.

use std::sync::OnceLock;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

pub const WM_APP_TRAY: u32 = WM_APP + 4;
pub const CMD_TOGGLE: usize = 1;
pub const CMD_SETTINGS: usize = 2;
pub const CMD_QUIT: usize = 3;

static ICON: OnceLock<isize> = OnceLock::new();

/// A small rounded "M" glyph drawn at runtime (no image assets to ship).
pub fn icon() -> HICON {
    HICON(*ICON.get_or_init(|| unsafe { draw_icon(32) }.0 as isize) as _)
}

unsafe fn draw_icon(size: i32) -> HICON {
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let color = CreateCompatibleBitmap(screen, size, size);
        let mask = CreateBitmap(size, size, 1, 1, None);
        let pen = CreatePen(PS_NULL, 0, COLORREF(0));

        // Mask: white = transparent, black = opaque rounded square.
        let old = SelectObject(dc, mask.into());
        let white = CreateSolidBrush(COLORREF(0xFFFFFF));
        let black = CreateSolidBrush(COLORREF(0));
        FillRect(dc, &RECT { left: 0, top: 0, right: size, bottom: size }, white);
        let ob = SelectObject(dc, black.into());
        let op = SelectObject(dc, pen.into());
        let _ = RoundRect(dc, 1, 1, size, size, size / 3, size / 3);

        // Color: accent rounded square with a white serif "M".
        SelectObject(dc, color.into());
        FillRect(dc, &RECT { left: 0, top: 0, right: size, bottom: size }, black);
        let accent = CreateSolidBrush(COLORREF(0x00B85F00)); // #005FB8
        SelectObject(dc, accent.into());
        let _ = RoundRect(dc, 1, 1, size, size, size / 3, size / 3);
        let font = CreateFontW(-(size * 3 / 4), 0, 0, 0, 600, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, 0, w!("Georgia"));
        let of = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0xFFFFFF));
        let mut t: Vec<u16> = "M".encode_utf16().collect();
        let mut rc = RECT { left: 0, top: 0, right: size, bottom: size };
        DrawTextW(dc, &mut t, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

        SelectObject(dc, of);
        SelectObject(dc, ob);
        SelectObject(dc, op);
        SelectObject(dc, old);
        let info = ICONINFO { fIcon: TRUE, hbmMask: mask, hbmColor: color, ..Default::default() };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        for o in [font.into(), accent.into(), white.into(), black.into(), pen.into(), color.into(), mask.into()] {
            let _: HGDIOBJ = o;
            let _ = DeleteObject(o);
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        icon
    }
}

fn nid(hwnd: HWND, tip: &str) -> NOTIFYICONDATAW {
    let mut n = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: icon(),
        ..Default::default()
    };
    for (i, c) in tip.encode_utf16().take(127).enumerate() {
        n.szTip[i] = c;
    }
    n.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    n
}

pub fn add(hwnd: HWND, tip: &str) {
    unsafe {
        let n = nid(hwnd, tip);
        let _ = Shell_NotifyIconW(NIM_ADD, &n);
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &n);
    }
}

pub fn set_tip(hwnd: HWND, tip: &str) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid(hwnd, tip));
    }
}

pub fn remove(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid(hwnd, ""));
    }
}

pub fn menu(hwnd: HWND, enabled: bool) -> usize {
    unsafe {
        let m = CreatePopupMenu().unwrap();
        let _ = AppendMenuW(m, MF_STRING, CMD_TOGGLE, if enabled { w!("Pause Margin") } else { w!("Resume Margin") });
        let _ = AppendMenuW(m, MF_STRING, CMD_SETTINGS, w!("Settings…"));
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(m, MF_STRING, CMD_QUIT, w!("Quit"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, Some(0), hwnd, None);
        let _ = DestroyMenu(m);
        cmd.0 as usize
    }
}
