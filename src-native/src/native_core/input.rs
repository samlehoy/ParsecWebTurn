//! Physical USB usages, as in lowlat examples/client/keys.h. Reuse our wire
//! framing, viewport and HID adapter; characters remain the host layout's.
use crate::{
    bud_attempt::Attempt,
    window::{Event, Window},
};
use anyhow::Result;

fn usage(code: &str) -> Option<i32> {
    let bytes = code.as_bytes();
    if code.starts_with("Key") && bytes.len() == 4 && bytes[3].is_ascii_uppercase() {
        return Some(i32::from(bytes[3] - b'A') + 4);
    }
    if code.starts_with("Digit") && bytes.len() == 6 && bytes[5].is_ascii_digit() {
        return Some(if bytes[5] == b'0' {
            39
        } else {
            i32::from(bytes[5] - b'1') + 30
        });
    }
    if let Some(n) = code.strip_prefix('F').and_then(|s| s.parse::<i32>().ok()) {
        if (1..=12).contains(&n) {
            return Some(57 + n);
        }
        if (13..=24).contains(&n) {
            return Some(91 + n);
        }
    }
    Some(match code {
        "Enter" => 40,
        "Escape" => 41,
        "Backspace" => 42,
        "Tab" => 43,
        "Space" => 44,
        "Minus" => 45,
        "Equal" => 46,
        "BracketLeft" => 47,
        "BracketRight" => 48,
        "Backslash" => 49,
        "Semicolon" => 51,
        "Quote" => 52,
        "Backquote" => 53,
        "Comma" => 54,
        "Period" => 55,
        "Slash" => 56,
        "CapsLock" => 57,
        "PrintScreen" => 70,
        "ScrollLock" => 71,
        "Pause" => 72,
        "Insert" => 73,
        "Home" => 74,
        "PageUp" => 75,
        "Delete" => 76,
        "End" => 77,
        "PageDown" => 78,
        "ArrowRight" => 79,
        "ArrowLeft" => 80,
        "ArrowDown" => 81,
        "ArrowUp" => 82,
        "NumLock" => 83,
        "NumpadDivide" => 84,
        "NumpadMultiply" => 85,
        "NumpadSubtract" => 86,
        "NumpadAdd" => 87,
        "NumpadEnter" => 88,
        "Numpad1" => 89,
        "Numpad2" => 90,
        "Numpad3" => 91,
        "Numpad4" => 92,
        "Numpad5" => 93,
        "Numpad6" => 94,
        "Numpad7" => 95,
        "Numpad8" => 96,
        "Numpad9" => 97,
        "Numpad0" => 98,
        "NumpadDecimal" => 99,
        "IntlBackslash" => 100,
        "ContextMenu" => 101,
        "NumpadEqual" => 103,
        "ControlLeft" => 224,
        "ShiftLeft" => 225,
        "AltLeft" => 226,
        "MetaLeft" => 227,
        "ControlRight" => 228,
        "ShiftRight" => 229,
        "AltRight" => 230,
        "MetaRight" => 231,
        _ => return None,
    })
}

pub fn send(attempt: &Attempt, window: &Window, event: &Event) -> Result<()> {
    use crate::control::header;
    let viewport = window.presented_viewport();
    let packet = match event {
        Event::Key(down, code, mods) => {
            let Some(usage) = usage(code) else {
                return Ok(());
            };
            header(0, usage, *mods, i32::from(*down))
        }
        Event::Button(down, button, ..) => header(1, *button, i32::from(*down), 0),
        Event::Scroll(x, y) => header(2, *x, *y, 0),
        Event::Motion(x, y) => {
            let Some((x, y)) = viewport.and_then(|v| v.map(*x, *y)) else {
                return Ok(());
            };
            header(3, 0, x, y)
        }
        Event::RelativeMotion(x, y) => header(3, 1, *x, *y),
        Event::RelativeMode(active) if attempt.hid_mouse_enabled() => {
            header(3, 3, i32::from(*active), 0)
        }
        Event::Focus(false) | Event::HidInputRelease => header(24, 0, 0, 0),
        _ => return Ok(()),
    };
    attempt.native_input(&packet, viewport)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_usages_preserve_hungarian_iso_key_and_modifiers() {
        for (code, expected) in [
            ("KeyY", 28),
            ("KeyZ", 29),
            ("IntlBackslash", 100),
            ("AltRight", 230),
            ("ControlRight", 228),
            ("Digit0", 39),
            ("F24", 115),
        ] {
            assert_eq!(usage(code), Some(expected));
        }
        assert_eq!(usage("unrecognized"), None);
    }
}
