//! Suggestions drawn on the text itself, over the app being typed in:
//! - mistakes get a solid underline and a light highlight (red = correctness,
//!   blue = clarity), like QuillBot; the hovered word's highlight gets stronger;
//! - the predicted next words appear as grey "ghost" text right after the caret.
//!
//! A layered, click-through, non-activating window: it only paints. Mouse clicks
//! and keys go straight to the app underneath. Hovering is detected by the main
//! thread (cursor position vs. `hits`).

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

const CLASS: PCWSTR = w!("MarginUnderlines");

/// Windows "critical" red, for spelling/grammar/punctuation.
pub const RED: u32 = 0xD1_34_38;
/// Windows accent blue, for clarity (Improve) suggestions.
pub const BLUE: u32 = 0x0F_6C_BD;

pub struct Overlay {
    pub hwnd: HWND,
    /// (index into the issue list, rectangle of the highlighted text) in screen pixels.
    pub hits: Vec<(usize, RECT)>,
    /// Where the ghost prediction is drawn, if any.
    pub ghost: Option<RECT>,
    pub owner: HWND,
    /// What was last drawn, so hovering can redraw with a stronger highlight.
    drawn: Vec<(usize, RECT, u32)>,
    ghost_src: Option<(RECT, String)>,
    hovered: Option<usize>,
}

pub fn register(inst: HINSTANCE) {
    unsafe {
        let wc = WNDCLASSW { lpfnWndProc: Some(proc_), hInstance: inst, lpszClassName: CLASS, ..Default::default() };
        RegisterClassW(&wc);
    }
}

unsafe extern "system" fn proc_(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if m == WM_MOUSEACTIVATE {
            return LRESULT(MA_NOACTIVATE as isize);
        }
        DefWindowProcW(h, m, w, l)
    }
}

/// Premultiplied ARGB for an RGB colour at `alpha` (0-255).
fn argb(rgb: u32, alpha: u32) -> u32 {
    let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    (alpha << 24) | ((r * alpha / 255) << 16) | ((g * alpha / 255) << 8) | (b * alpha / 255)
}

impl Overlay {
    pub fn create(inst: HINSTANCE) -> Overlay {
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS,
                w!(""),
                WS_POPUP,
                0, 0, 1, 1,
                None, None, Some(inst), None,
            )
            .unwrap()
        };
        Overlay { hwnd, hits: vec![], ghost: None, owner: HWND::default(), drawn: vec![], ghost_src: None, hovered: None }
    }

    pub fn visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn hide(&mut self) {
        self.hits.clear();
        self.ghost = None;
        self.drawn.clear();
        self.ghost_src = None;
        self.hovered = None;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.set_title();
    }

    /// Which issue (if any) is under this screen point: the highlighted word plus a
    /// few pixels below it, so the underline is easy to hover.
    pub fn hit(&self, pt: POINT) -> Option<(usize, RECT)> {
        self.hits.iter().copied().find(|(_, r)| pt.x >= r.left && pt.x <= r.right && pt.y >= r.top && pt.y <= r.bottom + 4)
    }

    /// Exposes what is drawn in the (invisible) window title, for automated tests.
    fn set_title(&self) {
        let mut t: String = self.hits.iter().map(|(i, r)| format!("{i}:{},{},{},{};", r.left, r.top, r.right, r.bottom)).collect();
        if let Some(g) = self.ghost {
            t += &format!("ghost:{},{},{},{};", g.left, g.top, g.right, g.bottom);
        }
        let w: Vec<u16> = t.encode_utf16().chain([0]).collect();
        unsafe {
            let _ = SetWindowTextW(self.hwnd, PCWSTR(w.as_ptr()));
        }
    }

    /// Draw highlights for `hits` (index, rect, colour) and an optional ghost
    /// prediction `(caret rect, text)`.
    pub fn show(&mut self, owner: HWND, hits: Vec<(usize, RECT, u32)>, ghost: Option<(RECT, String)>) {
        let hits: Vec<(usize, RECT, u32)> = hits.into_iter().filter(|(_, r, _)| r.right > r.left && r.bottom > r.top).collect();
        let ghost = ghost.filter(|(c, t)| c.bottom > c.top && !t.trim().is_empty());
        if hits.is_empty() && ghost.is_none() {
            self.hide();
            return;
        }
        self.owner = owner;
        self.drawn = hits;
        self.ghost_src = ghost;
        self.render();
    }

    /// Remove only the grey prediction (the user typed past it); keep underlines.
    pub fn drop_ghost(&mut self) {
        if self.ghost_src.take().is_some() {
            self.ghost = None;
            if self.drawn.is_empty() {
                self.hide();
            } else {
                self.render();
            }
        }
    }

    /// Emphasize the highlight of the issue under the mouse (None = no hover).
    pub fn set_hover(&mut self, i: Option<usize>) {
        if self.hovered != i && (!self.drawn.is_empty() || self.ghost_src.is_some()) {
            self.hovered = i;
            self.render();
        }
    }

    fn render(&mut self) {
        let hits = self.drawn.clone();
        let ghost = self.ghost_src.clone();
        unsafe {
            let dpi = GetDpiForWindow(self.hwnd).max(96) as i32;
            let screen = GetDC(None);

            // Ghost text: sized from the caret height so it matches the app's text.
            let mut ghost_rect = None;
            let mut ghost_font = HFONT::default();
            if let Some((c, text)) = &ghost {
                let line_h = c.bottom - c.top;
                let px = ((line_h as f32) * 0.8).round().max(10.0) as i32;
                ghost_font = CreateFontW(-px, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, 0, w!("Segoe UI"));
                let old = SelectObject(screen, ghost_font.into());
                let wtext: Vec<u16> = text.encode_utf16().collect();
                let mut sz = SIZE::default();
                let _ = GetTextExtentPoint32W(screen, &wtext, &mut sz);
                SelectObject(screen, old);
                let left = c.right + 1;
                ghost_rect = Some(RECT { left, top: c.top, right: left + sz.cx + 2, bottom: c.bottom });
            }

            let pad = 4;
            let all: Vec<RECT> = hits.iter().map(|h| h.1).chain(ghost_rect).collect();
            let bx = RECT {
                left: all.iter().map(|r| r.left).min().unwrap() - pad,
                top: all.iter().map(|r| r.top).min().unwrap() - pad,
                right: all.iter().map(|r| r.right).max().unwrap() + pad,
                bottom: all.iter().map(|r| r.bottom).max().unwrap() + pad,
            };
            let (w, h) = (bx.right - bx.left, bx.bottom - bx.top);
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), bx.left, bx.top, w, h, SWP_NOACTIVATE);

            // 32-bit premultiplied ARGB canvas, fully transparent.
            let mut px = vec![0u32; (w * h) as usize];
            let thick = (2 * dpi / 96).max(2);
            for (i, r, color) in &hits {
                // Highlighter tint over the word (text stays readable through it).
                let tint = argb(*color, if self.hovered == Some(*i) { 0x55 } else { 0x22 });
                for y in r.top..r.bottom {
                    for x in r.left..r.right {
                        let (lx, ly) = (x - bx.left, y - bx.top);
                        px[(ly * w + lx) as usize] = tint;
                    }
                }
                // Solid underline along the bottom edge, like QuillBot.
                let line = argb(*color, 0xFF);
                for y in (r.bottom - thick)..r.bottom {
                    for x in r.left..r.right {
                        let (lx, ly) = (x - bx.left, y - bx.top);
                        if (0..h).contains(&ly) && (0..w).contains(&lx) {
                            px[(ly * w + lx) as usize] = line;
                        }
                    }
                }
            }

            let mem = CreateCompatibleDC(Some(screen));
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            if let Ok(dib) = CreateDIBSection(Some(mem), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
                let buf = std::slice::from_raw_parts_mut(bits as *mut u32, px.len());
                buf.copy_from_slice(&px);
                let old = SelectObject(mem, dib.into());
                if let (Some((_, text)), Some(gr)) = (&ghost, ghost_rect) {
                    // GDI can't write alpha: draw white-on-transparent, then turn the
                    // drawn coverage into alpha for a soft grey.
                    let gr_local = RECT { left: gr.left - bx.left, top: gr.top - bx.top, right: gr.right - bx.left, bottom: gr.bottom - bx.top };
                    for y in gr_local.top.max(0)..gr_local.bottom.min(h) {
                        for x in gr_local.left.max(0)..gr_local.right.min(w) {
                            buf[(y * w + x) as usize] = 0;
                        }
                    }
                    let of = SelectObject(mem, ghost_font.into());
                    SetBkMode(mem, TRANSPARENT);
                    SetTextColor(mem, COLORREF(0x00FF_FFFF));
                    let mut wtext: Vec<u16> = text.encode_utf16().collect();
                    let mut r = gr_local;
                    DrawTextW(mem, &mut wtext, &mut r, DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_NOPREFIX);
                    SelectObject(mem, of);
                    let _ = GdiFlush();
                    for y in gr_local.top.max(0)..gr_local.bottom.min(h) {
                        for x in gr_local.left.max(0)..gr_local.right.min(w) {
                            let p = buf[(y * w + x) as usize];
                            let cov = ((p >> 16) & 0xFF).max((p >> 8) & 0xFF).max(p & 0xFF);
                            buf[(y * w + x) as usize] = if cov == 0 { 0 } else { argb(0x80_80_80, cov * 0xC0 / 255) };
                        }
                    }
                }
                let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
                let pos = POINT { x: bx.left, y: bx.top };
                let size = SIZE { cx: w, cy: h };
                let src = POINT { x: 0, y: 0 };
                let _ = UpdateLayeredWindow(self.hwnd, Some(screen), Some(&pos), Some(&size), Some(mem), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
                SelectObject(mem, old);
                let _ = DeleteObject(dib.into());
            }
            if !ghost_font.is_invalid() {
                let _ = DeleteObject(ghost_font.into());
            }
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            self.ghost = ghost_rect;
        }
        self.hits = hits.into_iter().map(|(i, r, _)| (i, r)).collect();
        self.set_title();
    }
}

/// Global overlay, owned by the main thread.
pub static mut OVERLAY: Option<Overlay> = None;

#[allow(static_mut_refs)]
pub fn get() -> &'static mut Overlay {
    unsafe { OVERLAY.as_mut().expect("overlay created") }
}
