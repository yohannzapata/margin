//! Restrained Windows 11 look: system light/dark, Segoe UI, accent sparingly.

use std::sync::Mutex;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Registry::*;
use windows::core::w;

pub struct Palette {
    pub bg: COLORREF,
    pub text: COLORREF,
    pub muted: COLORREF,
    pub border: COLORREF,
    pub selected: COLORREF,
    pub accent: COLORREF,
    pub accent_text: COLORREF,
}

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

pub fn dark_mode() -> bool {
    let mut v: u32 = 1;
    let mut sz = 4u32;
    unsafe {
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut _ as _),
            Some(&mut sz),
        );
    }
    v == 0
}

pub fn palette() -> Palette {
    if dark_mode() {
        Palette {
            bg: rgb(0x2C, 0x2C, 0x2C),
            text: rgb(0xF3, 0xF3, 0xF3),
            muted: rgb(0x9E, 0x9E, 0x9E),
            border: rgb(0x40, 0x40, 0x40),
            selected: rgb(0x38, 0x38, 0x38),
            accent: rgb(0x4C, 0xC2, 0xFF),
            accent_text: rgb(0x99, 0xEB, 0xFF),
        }
    } else {
        Palette {
            bg: rgb(0xFF, 0xFF, 0xFF),
            text: rgb(0x1A, 0x1A, 0x1A),
            muted: rgb(0x6E, 0x6E, 0x6E),
            border: rgb(0xE0, 0xE0, 0xE0),
            selected: rgb(0xF0, 0xF4, 0xF9),
            accent: rgb(0x00, 0x5F, 0xB8),
            accent_text: rgb(0x00, 0x5F, 0xB8),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Fonts {
    pub body: HFONT,
    pub bold: HFONT,
    pub small: HFONT,
    pub strike: HFONT,
}

static FONTS: Mutex<Vec<(i32, isize, isize, isize, isize)>> = Mutex::new(Vec::new());

fn make(px: i32, weight: i32) -> HFONT {
    make_ex(px, weight, false)
}

fn make_ex(px: i32, weight: i32, strike: bool) -> HFONT {
    unsafe {
        CreateFontW(-px, 0, 0, 0, weight, 0, 0, strike as u32, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Segoe UI"))
    }
}

/// Fonts for a DPI, created once and cached.
pub fn fonts(dpi: i32) -> Fonts {
    let mut cache = FONTS.lock().unwrap();
    if let Some(&(_, a, b, c, d)) = cache.iter().find(|f| f.0 == dpi) {
        return Fonts { body: HFONT(a as _), bold: HFONT(b as _), small: HFONT(c as _), strike: HFONT(d as _) };
    }
    let body = make(14 * dpi / 96, 400);
    let bold = make(14 * dpi / 96, 600);
    let small = make(12 * dpi / 96, 400);
    let strike = make_ex(14 * dpi / 96, 400, true);
    cache.push((dpi, body.0 as isize, bold.0 as isize, small.0 as isize, strike.0 as isize));
    Fonts { body, bold, small, strike }
}

pub static BADGE: Mutex<String> = Mutex::new(String::new());

/// "On this PC" / "Sent to <host>": always visible so users know where text goes.
pub fn privacy_badge() -> String {
    BADGE.lock().unwrap().clone()
}
