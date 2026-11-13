//! Applying an accepted suggestion: select the changed tail with Shift+Left,
//! then type the replacement as Unicode input. This works in classic edit
//! controls, rich editors and web contenteditables alike, and keeps the
//! target app's own undo history (Ctrl+Z reverts the suggestion).

use crate::hook::INJECT_MAGIC;
use crate::text::EditPlan;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

fn key(vk: VIRTUAL_KEY, up: bool, extended: bool) -> INPUT {
    let mut flags = if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) };
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: flags, dwExtraInfo: INJECT_MAGIC, ..Default::default() } },
    }
}

fn unit(u: u16, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_UNICODE;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wScan: u, dwFlags: flags, dwExtraInfo: INJECT_MAGIC, ..Default::default() } },
    }
}

pub fn apply(plan: &EditPlan) -> bool {
    let mut inputs = Vec::with_capacity(plan.select * 2 + plan.insert.len() * 2 + 4);
    // Make sure no physical modifier leaks into our input.
    for vk in [VK_LSHIFT, VK_RSHIFT, VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RMENU] {
        if unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000 != 0 {
            inputs.push(key(vk, true, false));
        }
    }
    if plan.select > 0 {
        inputs.push(key(VK_SHIFT, false, false));
        for _ in 0..plan.select {
            inputs.push(key(VK_LEFT, false, true));
            inputs.push(key(VK_LEFT, true, true));
        }
        inputs.push(key(VK_SHIFT, true, false));
    }
    let text = plan.insert.replace(['\r', '\n'], " ");
    if text.is_empty() && plan.select > 0 {
        inputs.push(key(VK_BACK, false, false));
        inputs.push(key(VK_BACK, true, false));
    }
    for u in text.encode_utf16() {
        inputs.push(unit(u, false));
        inputs.push(unit(u, true));
    }
    if inputs.is_empty() {
        return true;
    }
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    sent as usize == inputs.len()
}

/// Type `text` over the current selection (Backspace if it is empty).
pub fn replace_selection(text: &str) -> bool {
    let mut inputs = vec![];
    if text.is_empty() {
        inputs.push(key(VK_BACK, false, false));
        inputs.push(key(VK_BACK, true, false));
    }
    for u in text.replace(['\r', '\n'], " ").encode_utf16() {
        inputs.push(unit(u, false));
        inputs.push(unit(u, true));
    }
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    sent as usize == inputs.len()
}
