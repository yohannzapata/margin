//! The suggestion popup: a small, non-activating window next to the caret.
//! It never takes focus, so the user keeps typing in their app.

use crate::engine::Suggestion;
use crate::inference::Style;
use crate::theme::{self, Palette};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

pub struct Popup {
    pub hwnd: HWND,
    pub items: Vec<Suggestion>,
    pub note: Option<String>,
    pub selected: usize,
    pub owner: HWND,
    pub busy: bool,
    /// Compact mode: a strip of pills under a highlighted word ("that's" | "Fix all"),
    /// instead of the full list.
    pub compact: bool,
    /// Style buttons across the top of the list (when Rephrase options are shown).
    pub styles: Option<Style>,
}

/// Pill text: the fix itself, or the row's action.
fn pill_label(it: &Suggestion) -> (String, u8) {
    match &it.issue {
        Some(h) if h.repl.trim().is_empty() => (format!("Remove \"{}\"", h.orig.trim()), 0),
        Some(h) => (h.repl.trim().to_string(), 1),
        None => (it.tag().to_string(), 0),
    }
}

const CLASS: PCWSTR = w!("MarginPopup");
const MAX_W: i32 = 560;
const ROW_H: i32 = 30;
const TAG_W: i32 = 92;

/// Display pieces of a row: (text, style) with style 0 = normal, 1 = changed, 2 = struck out.
fn pieces(it: &Suggestion) -> Vec<(String, u8)> {
    match &it.issue {
        Some(h) if h.repl.is_empty() => vec![(h.orig.trim().to_string(), 2)],
        Some(h) => vec![(h.orig.trim().to_string(), 2), ("  →  ".to_string(), 0), (h.repl.trim().to_string(), 1)],
        None => it.runs.iter().map(|r| (r.text.clone(), r.changed as u8)).collect(),
    }
}

pub fn register(inst: HINSTANCE) {
    unsafe {
        let wc = WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(wndproc),
            hInstance: inst,
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_HAND).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
}

impl Popup {
    pub fn create(inst: HINSTANCE) -> Popup {
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS,
                w!("Margin suggestions"),
                WS_POPUP,
                0, 0, 10, 10,
                None, None, Some(inst), None,
            )
            .unwrap()
        };
        unsafe {
            let pref = DWMWCP_ROUNDSMALL;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as _, 4);
        }
        Popup { hwnd, items: vec![], note: None, selected: 0, owner: HWND::default(), busy: false, compact: false, styles: None }
    }

    pub fn visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn hide(&mut self) {
        self.items.clear();
        self.note = None;
        self.selected = 0;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        crate::hook::POPUP_VISIBLE.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    fn dpi(&self) -> i32 {
        unsafe { GetDpiForWindow(self.hwnd).max(96) as i32 }
    }

    fn s(&self, v: i32) -> i32 {
        v * self.dpi() / 96
    }

    /// Show (or resize) the popup anchored to the caret rectangle.
    pub fn show_at(&mut self, caret: RECT) {
        if self.items.is_empty() && self.note.is_none() {
            self.hide();
            return;
        }
        if self.selected >= self.items.len().max(1) {
            self.selected = 0;
        }
        // Row summary in the (invisible) window title: used by automated tests.
        let summary: Vec<String> = self.items.iter().map(|it| format!("{}: {}", it.tag(), pieces(it).iter().map(|p| p.0.as_str()).collect::<String>())).collect();
        let t: Vec<u16> = summary.join(" | ").encode_utf16().chain([0]).collect();
        unsafe {
            let _ = SetWindowTextW(self.hwnd, PCWSTR(t.as_ptr()));
        }
        unsafe {
            // Position first so DPI is correct for the monitor we land on.
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), caret.left, caret.bottom, 10, 10, SWP_NOACTIVATE);
        }
        let (w, h) = if self.compact { self.measure_pills() } else { self.measure() };
        let gap = if self.compact { self.s(3) } else { self.s(6) };
        let mut x = caret.left - if self.compact { self.s(4) } else { self.s(12) };
        let mut y = caret.bottom + gap;
        unsafe {
            let mon = MonitorFromPoint(POINT { x: caret.left, y: caret.bottom }, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let wa = mi.rcWork;
            if y + h > wa.bottom {
                y = caret.top - gap - h; // flip above the caret
            }
            x = x.clamp(wa.left + 4, (wa.right - w - 4).max(wa.left));
            y = y.max(wa.top);
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let _ = InvalidateRect(Some(self.hwnd), None, true);
        }
        crate::hook::POPUP_VISIBLE.store(!self.items.is_empty(), std::sync::atomic::Ordering::Relaxed);
    }

    pub fn refresh(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn rows(&self) -> i32 {
        (self.items.len() + self.note.is_some() as usize) as i32
    }

    fn measure(&self) -> (i32, i32) {
        let fonts = theme::fonts(self.dpi());
        let mut w = self.s(220);
        unsafe {
            let dc = GetDC(Some(self.hwnd));
            let old = SelectObject(dc, fonts.body.into());
            for it in &self.items {
                // Measure each piece in the font it is drawn with (bold is wider).
                let mut tw = 0;
                for (t, style) in pieces(it) {
                    SelectObject(dc, match style { 1 => fonts.bold.into(), 2 => fonts.strike.into(), _ => fonts.body.into() });
                    tw += text_width(dc, &t);
                }
                SelectObject(dc, fonts.body.into());
                w = w.max(tw + self.s(TAG_W + 32));
            }
            if let Some(n) = &self.note {
                w = w.max(text_width(dc, n) + self.s(28));
            }
            SelectObject(dc, old);
            ReleaseDC(Some(self.hwnd), dc);
        }
        let w = w.max(self.style_rects().last().map(|r| r.right + self.s(8)).unwrap_or(0) * (self.header_h() > 0) as i32);
        let w = w.min(self.s(MAX_W));
        let h = self.header_h() + self.s(8) + self.rows() * self.s(ROW_H) + self.s(24) + self.s(4);
        (w, h)
    }

    /// Pill rectangles (client coordinates) in compact mode.
    fn pill_rects(&self) -> Vec<RECT> {
        let fonts = theme::fonts(self.dpi());
        let mut out = vec![];
        unsafe {
            let dc = GetDC(Some(self.hwnd));
            let mut x = self.s(4);
            for it in &self.items {
                let (t, style) = pill_label(it);
                let old = SelectObject(dc, if style == 1 { fonts.bold.into() } else { fonts.body.into() });
                let w = text_width(dc, &t) + self.s(22);
                SelectObject(dc, old);
                out.push(RECT { left: x, top: self.s(4), right: x + w, bottom: self.s(32) });
                x += w + self.s(4);
            }
            ReleaseDC(Some(self.hwnd), dc);
        }
        out
    }

    fn measure_pills(&self) -> (i32, i32) {
        let r = self.pill_rects();
        (r.last().map(|x| x.right).unwrap_or(0) + self.s(4), self.s(36))
    }

    fn item_at(&self, x: i32, y: i32) -> Option<usize> {
        if self.compact {
            return self.pill_rects().iter().position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom);
        }
        self.row_at(y)
    }

    /// Height of the style-button strip (0 when not shown).
    fn header_h(&self) -> i32 {
        if self.styles.is_some() && !self.compact { self.s(36) } else { 0 }
    }

    /// Style-button rectangles (client coordinates).
    fn style_rects(&self) -> Vec<RECT> {
        let fonts = theme::fonts(self.dpi());
        let mut out = vec![];
        unsafe {
            let dc = GetDC(Some(self.hwnd));
            let old = SelectObject(dc, fonts.small.into());
            let mut x = self.s(8);
            for st in Style::ALL {
                let w = text_width(dc, st.label()) + self.s(18);
                out.push(RECT { left: x, top: self.s(6), right: x + w, bottom: self.s(30) });
                x += w + self.s(4);
            }
            SelectObject(dc, old);
            ReleaseDC(Some(self.hwnd), dc);
        }
        out
    }

    /// The style button at a point, if any.
    fn style_at(&self, x: i32, y: i32) -> Option<Style> {
        if self.header_h() == 0 {
            return None;
        }
        self.style_rects().iter().position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|i| Style::ALL[i])
    }

    fn row_at(&self, y: i32) -> Option<usize> {
        let y = y - self.header_h();
        let top = self.s(4);
        if y < top {
            return None;
        }
        let i = ((y - top) / self.s(ROW_H)) as usize;
        (i < self.items.len()).then_some(i)
    }

    unsafe fn paint(&self, hdc: HDC) {
        unsafe {
            let p: Palette = theme::palette();
            let fonts = theme::fonts(self.dpi());
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            // Double-buffer to avoid flicker while results stream in.
            let mem = CreateCompatibleDC(Some(hdc));
            let bmp = CreateCompatibleBitmap(hdc, rc.right, rc.bottom);
            let oldbmp = SelectObject(mem, bmp.into());
            fill(mem, &rc, p.bg);
            frame(mem, &rc, p.border);
            SetBkMode(mem, TRANSPARENT);
            if self.compact {
                for (i, (it, r)) in self.items.iter().zip(self.pill_rects()).enumerate() {
                    if i == self.selected {
                        fill_round(mem, &r, p.selected, self.s(4));
                    }
                    let (t, style) = pill_label(it);
                    SelectObject(mem, if style == 1 { fonts.bold.into() } else { fonts.body.into() });
                    SetTextColor(mem, if style == 1 { p.accent_text } else { p.text });
                    let mut rr = r;
                    draw(mem, &t, &mut rr, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                }
                let _ = BitBlt(hdc, 0, 0, rc.right, rc.bottom, Some(mem), 0, 0, SRCCOPY);
                SelectObject(mem, oldbmp);
                let _ = DeleteObject(bmp.into());
                let _ = DeleteDC(mem);
                return;
            }

            let pad = self.s(12);
            if let Some(current) = self.styles.filter(|_| self.header_h() > 0) {
                SelectObject(mem, fonts.small.into());
                for (st, r) in Style::ALL.iter().zip(self.style_rects()) {
                    if *st == current {
                        fill_round(mem, &r, p.accent, self.s(4));
                        SetTextColor(mem, p.bg);
                    } else {
                        fill_round(mem, &r, p.selected, self.s(4));
                        SetTextColor(mem, p.text);
                    }
                    let mut rr = r;
                    draw(mem, st.label(), &mut rr, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                }
                let sep = RECT { left: 0, top: self.header_h() - 1, right: rc.right, bottom: self.header_h() };
                fill(mem, &sep, p.border);
            }
            let mut y = self.header_h() + self.s(4);
            for (i, it) in self.items.iter().enumerate() {
                let row = RECT { left: self.s(4), top: y, right: rc.right - self.s(4), bottom: y + self.s(ROW_H) };
                if i == self.selected {
                    fill_round(mem, &row, p.selected, self.s(4));
                    // Accent bar, like Windows 11 list selection.
                    let bar = RECT { left: row.left, top: row.top + self.s(8), right: row.left + self.s(3), bottom: row.bottom - self.s(8) };
                    fill_round(mem, &bar, p.accent, self.s(2));
                }
                SelectObject(mem, fonts.small.into());
                SetTextColor(mem, p.muted);
                let mut tag = RECT { left: pad, top: row.top, right: pad + self.s(TAG_W), bottom: row.bottom };
                draw(mem, it.tag(), &mut tag, DT_SINGLELINE | DT_VCENTER | DT_LEFT);

                let mut x = pad + self.s(TAG_W);
                let right = row.right - self.s(8);
                let baseline_rc = |x: i32| RECT { left: x, top: row.top, right, bottom: row.bottom };
                for (text, style) in pieces(it) {
                    SelectObject(mem, match style { 1 => fonts.bold.into(), 2 => fonts.strike.into(), _ => fonts.body.into() });
                    SetTextColor(mem, match style {
                        1 if it.kind != crate::engine::Kind::Rephrase => p.accent_text,
                        2 => p.muted,
                        _ => p.text,
                    });
                    let wpx = text_width(mem, &text);
                    let mut r = baseline_rc(x);
                    if x + wpx > right {
                        draw(mem, &text, &mut r, DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS);
                        break;
                    }
                    draw(mem, &text, &mut r, DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_NOPREFIX);
                    x += wpx;
                }
                y += self.s(ROW_H);
            }
            if let Some(n) = &self.note {
                SelectObject(mem, fonts.body.into());
                SetTextColor(mem, p.muted);
                let mut r = RECT { left: pad, top: y, right: rc.right - pad, bottom: y + self.s(ROW_H) };
                draw(mem, n, &mut r, DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS);
                y += self.s(ROW_H);
            }
            // Footer: shortcuts, and whether text stays on this PC.
            let sep = RECT { left: 0, top: y + self.s(4), right: rc.right, bottom: y + self.s(4) + 1.max(self.s(1)) };
            fill(mem, &sep, p.border);
            SelectObject(mem, fonts.small.into());
            SetTextColor(mem, p.muted);
            let hint = if self.header_h() > 0 {
                "Tab accept   Alt+↑↓ choose   Alt+←→ style".to_string()
            } else if self.items.is_empty() {
                "Esc to close".to_string()
            } else if self.items.len() > 1 {
                "Tab accept   Alt+↑↓ choose   Esc dismiss".to_string()
            } else {
                "Tab accept   Esc dismiss".to_string()
            };
            let mut fr = RECT { left: pad, top: y + self.s(5), right: rc.right - pad, bottom: rc.bottom - self.s(2) };
            draw(mem, &hint, &mut fr, DT_SINGLELINE | DT_VCENTER | DT_LEFT);
            let where_ = if self.busy { "…".to_string() } else { theme::privacy_badge() };
            draw(mem, &where_, &mut fr, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);

            let _ = BitBlt(hdc, 0, 0, rc.right, rc.bottom, Some(mem), 0, 0, SRCCOPY);
            SelectObject(mem, oldbmp);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem);
        }
    }
}

pub fn text_width(dc: HDC, s: &str) -> i32 {
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut sz = SIZE::default();
    unsafe {
        let _ = GetTextExtentPoint32W(dc, &w, &mut sz);
    }
    sz.cx
}

fn draw(dc: HDC, s: &str, rc: &mut RECT, fmt: DRAW_TEXT_FORMAT) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        DrawTextW(dc, &mut w, rc, fmt | DT_NOPREFIX);
    }
}

fn fill(dc: HDC, rc: &RECT, c: COLORREF) {
    unsafe {
        let b = CreateSolidBrush(c);
        FillRect(dc, rc, b);
        let _ = DeleteObject(b.into());
    }
}

fn fill_round(dc: HDC, rc: &RECT, c: COLORREF, r: i32) {
    unsafe {
        let b = CreateSolidBrush(c);
        let pen = CreatePen(PS_NULL, 0, COLORREF(0));
        let ob = SelectObject(dc, b.into());
        let op = SelectObject(dc, pen.into());
        let _ = RoundRect(dc, rc.left, rc.top, rc.right + 1, rc.bottom + 1, r * 2, r * 2);
        SelectObject(dc, ob);
        SelectObject(dc, op);
        let _ = DeleteObject(b.into());
        let _ = DeleteObject(pen.into());
    }
}

fn frame(dc: HDC, rc: &RECT, c: COLORREF) {
    unsafe {
        let b = CreateSolidBrush(c);
        FrameRect(dc, rc, b);
        let _ = DeleteObject(b.into());
    }
}

/// Global pointer to the popup state, owned by the main thread.
pub static mut POPUP: Option<Popup> = None;

#[allow(static_mut_refs)]
pub fn get() -> &'static mut Popup {
    unsafe { POPUP.as_mut().expect("popup created") }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                #[allow(static_mut_refs)]
                if let Some(p) = POPUP.as_ref() {
                    p.paint(hdc);
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = ((lp.0 & 0xFFFF) as i16 as i32, (lp.0 >> 16) as i16 as i32);
                let p = get();
                if let Some(i) = p.item_at(x, y) {
                    if i != p.selected {
                        p.selected = i;
                        p.refresh();
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = ((lp.0 & 0xFFFF) as i16 as i32, (lp.0 >> 16) as i16 as i32);
                if let Some(st) = get().style_at(x, y) {
                    let i = Style::ALL.iter().position(|s| *s == st).unwrap_or(0);
                    let owner = crate::hook::MAIN_HWND.load(std::sync::atomic::Ordering::Relaxed);
                    let _ = PostMessageW(Some(HWND(owner as *mut _)), crate::hook::WM_APP_ACTION, WPARAM(crate::hook::ACT_STYLE | i << 8), LPARAM(1));
                } else if let Some(i) = get().item_at(x, y) {
                    get().selected = i;
                    let owner = crate::hook::MAIN_HWND.load(std::sync::atomic::Ordering::Relaxed);
                    let _ = PostMessageW(Some(HWND(owner as *mut _)), crate::hook::WM_APP_ACTION, WPARAM(crate::hook::ACT_ACCEPT), LPARAM(1));
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
