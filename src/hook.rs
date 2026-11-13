//! Low-level keyboard hook on a dedicated thread.
//!
//! Privacy: the hook never translates or stores characters. It only tells the
//! main thread "a key was pressed" (so suggestions can be recomputed after a
//! pause) and intercepts the popup's own shortcuts while the popup is visible.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering::Relaxed};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_APP_TYPING: u32 = WM_APP + 1;
pub const WM_APP_ACTION: u32 = WM_APP + 2;

/// WM_APP_TYPING wParam flags (low 16 bits: virtual-key code).
pub const KEY_WORD_END: usize = 1 << 16;
pub const KEY_EDITING: usize = 1 << 17;

pub const ACT_ACCEPT: usize = 1;
pub const ACT_NEXT: usize = 2;
pub const ACT_PREV: usize = 3;
pub const ACT_DISMISS: usize = 4;
pub const ACT_ON_DEMAND: usize = 5;
/// Pick a Rephrase style; the style index is in bits 8.. of the action.
pub const ACT_STYLE: usize = 6;
pub const ACT_STYLE_PREV: usize = 7;
pub const ACT_STYLE_NEXT: usize = 8;

/// Marks keystrokes we inject so the hook ignores them.
pub const INJECT_MAGIC: usize = 0x4D52_474E; // "MRGN"

pub static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
pub static POPUP_VISIBLE: AtomicBool = AtomicBool::new(false);
pub static ENABLED: AtomicBool = AtomicBool::new(true);
/// On-demand shortcut: low 16 bits = vk, bit 16 ctrl, 17 shift, 18 alt, 19 win.
pub static ON_DEMAND: AtomicU32 = AtomicU32::new(0);

pub fn parse_shortcut(s: &str) -> Option<u32> {
    let mut mods = 0u32;
    let mut vk = 0u32;
    for part in s.split('+').map(|p| p.trim().to_lowercase()) {
        match part.as_str() {
            "ctrl" | "control" => mods |= 1 << 16,
            "shift" => mods |= 1 << 17,
            "alt" => mods |= 1 << 18,
            "win" => mods |= 1 << 19,
            "space" => vk = VK_SPACE.0 as u32,
            "enter" => vk = VK_RETURN.0 as u32,
            "tab" => vk = VK_TAB.0 as u32,
            "." | "period" => vk = VK_OEM_PERIOD.0 as u32,
            "," | "comma" => vk = VK_OEM_COMMA.0 as u32,
            "/" => vk = VK_OEM_2.0 as u32,
            ";" => vk = VK_OEM_1.0 as u32,
            p if p.len() == 1 && p.as_bytes()[0].is_ascii_alphanumeric() => vk = p.to_ascii_uppercase().as_bytes()[0] as u32,
            p if p.starts_with('f') && p[1..].parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)) => {
                vk = VK_F1.0 as u32 + p[1..].parse::<u32>().unwrap() - 1
            }
            _ => return None,
        }
    }
    (vk != 0 && mods != 0).then_some(mods | vk)
}

fn down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}

fn post(msg: u32, w: usize) {
    let h = MAIN_HWND.load(Relaxed);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), msg, WPARAM(w), LPARAM(0));
        }
    }
}

/// Swallowing a key while Alt is held makes some apps open their menu bar when
/// Alt is released; a dummy unassigned key press prevents that.
fn mask_alt() {
    let mk = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0xE8), dwFlags: flags, dwExtraInfo: INJECT_MAGIC, ..Default::default() } },
    };
    unsafe {
        SendInput(&[mk(KEYBD_EVENT_FLAGS(0)), mk(KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32);
    }
}

unsafe extern "system" fn proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 && ENABLED.load(Relaxed) {
            let k = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let is_down = wparam.0 as u32 == WM_KEYDOWN || wparam.0 as u32 == WM_SYSKEYDOWN;
            if k.dwExtraInfo != INJECT_MAGIC && is_down {
                let vk = VIRTUAL_KEY(k.vkCode as u16);
                let (ctrl, shift, alt, win) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU), down(VK_LWIN) || down(VK_RWIN));

                // On-demand shortcut works whether or not the popup is visible.
                let od = ON_DEMAND.load(Relaxed);
                if od != 0
                    && (od & 0xFFFF) == k.vkCode
                    && ((od >> 16) & 1 == 1) == ctrl
                    && ((od >> 17) & 1 == 1) == shift
                    && ((od >> 18) & 1 == 1) == alt
                    && ((od >> 19) & 1 == 1) == win
                {
                    post(WM_APP_ACTION, ACT_ON_DEMAND);
                    return LRESULT(1);
                }

                if POPUP_VISIBLE.load(Relaxed) {
                    let action = match vk {
                        VK_TAB if !ctrl && !alt && !shift && !win => Some(ACT_ACCEPT),
                        VK_DOWN if alt && !ctrl => Some(ACT_NEXT),
                        VK_UP if alt && !ctrl => Some(ACT_PREV),
                        VK_LEFT if alt && !ctrl => Some(ACT_STYLE_PREV),
                        VK_RIGHT if alt && !ctrl => Some(ACT_STYLE_NEXT),
                        VK_ESCAPE if !ctrl && !alt => Some(ACT_DISMISS),
                        _ => None,
                    };
                    if let Some(a) = action {
                        if alt {
                            mask_alt();
                        }
                        post(WM_APP_ACTION, a);
                        return LRESULT(1);
                    }
                }

                let modifier_only = matches!(
                    vk,
                    VK_SHIFT | VK_LSHIFT | VK_RSHIFT | VK_CONTROL | VK_LCONTROL | VK_RCONTROL | VK_MENU | VK_LMENU | VK_RMENU | VK_LWIN | VK_RWIN | VK_CAPITAL
                );
                if !modifier_only {
                    // Classify the key without recording which character it was:
                    // a word just ended (check now), or the text may move (hide underlines).
                    let packet_char = (vk == VK_PACKET).then(|| char::from_u32(k.scanCode)).flatten();
                    let boundary = matches!(vk, VK_SPACE | VK_OEM_PERIOD | VK_OEM_COMMA | VK_OEM_1 | VK_OEM_2)
                        || (vk.0 == b'1' as u16 && shift)
                        || packet_char.is_some_and(|c| c.is_whitespace() || ".,!?;:)".contains(c));
                    let editing = ctrl
                        || alt
                        || win
                        || matches!(vk, VK_BACK | VK_DELETE | VK_RETURN | VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT | VK_TAB | VK_ESCAPE | VK_INSERT);
                    post(WM_APP_TYPING, k.vkCode as usize | (boundary as usize) << 16 | (editing as usize) << 17);
                }
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

/// Install the hook on its own thread with a minimal message loop.
pub fn spawn() {
    std::thread::Builder::new()
        .name("margin-hook".into())
        .spawn(|| unsafe {
            let _ = windows::Win32::System::Threading::SetThreadPriority(
                windows::Win32::System::Threading::GetCurrentThread(),
                windows::Win32::System::Threading::THREAD_PRIORITY_TIME_CRITICAL,
            );
            let hmod = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok();
            let _hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), hmod.map(|h| h.into()), 0);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_parse() {
        assert_eq!(parse_shortcut("Ctrl+Shift+Space"), Some((1 << 16) | (1 << 17) | VK_SPACE.0 as u32));
        assert_eq!(parse_shortcut("Alt+F2"), Some((1 << 18) | VK_F2.0 as u32));
        assert_eq!(parse_shortcut("Space"), None); // needs a modifier
        assert_eq!(parse_shortcut("Ctrl+Bogus"), None);
    }
}
