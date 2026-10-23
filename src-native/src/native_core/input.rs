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

// Side-specific modifier bits from lowlat.h, not the browser's compact mask.
#[derive(Default)]
pub struct Input {
    modifiers: i32,
}
impl Input {
    fn frame(
        &mut self,
        event: &Event,
        viewport: Option<crate::viewport::Viewport>,
    ) -> Option<bytes::Bytes> {
        use crate::control::header;
        Some(match event {
            Event::Key(down, code, compact) => {
                let key = usage(code)?;
                if (224..=231).contains(&key) {
                    let bit = [64, 1, 256, 1024, 128, 2, 512, 2048][(key - 224) as usize];
                    if *down {
                        self.modifiers |= bit;
                    } else {
                        self.modifiers &= !bit;
                    }
                }
                let mut mods = self.modifiers;
                // A modifier may already be held when focus/session starts.
                for (source, bits, fallback) in
                    [(1, 3, 1), (2, 192, 64), (4, 768, 256), (8, 3072, 1024)]
                {
                    if compact & source != 0 && mods & bits == 0 {
                        mods |= fallback;
                    }
                }
                if compact & 16 != 0 {
                    mods |= 8192;
                }
                if compact & 32 != 0 {
                    mods |= 4096;
                }
                header(0, key, mods, i32::from(*down))
            }
            Event::Button(down, button, ..) if (0..=4).contains(button) => {
                header(1, *button + 1, i32::from(*down), 0)
            }
            // Window's vertical sign matches its browser callback; the native
            // lowlat/stock-host contract is Win32's wheel sign instead.
            Event::Scroll(x, y) => header(2, *x, y.saturating_neg(), 0),
            Event::Motion(x, y) => {
                let (x, y) = viewport?.map(*x, *y)?;
                header(3, 0, x, y)
            }
            Event::RelativeMotion(x, y) => header(3, 1, *x, *y),
            Event::RelativeMode(active) => header(3, 3, i32::from(*active), 0),
            Event::Focus(false) | Event::HidInputRelease => {
                self.modifiers = 0;
                header(24, 0, 0, 0)
            }
            _ => return None,
        })
    }
    pub fn send(&mut self, attempt: &Attempt, window: &Window, event: &Event) -> Result<()> {
        if matches!(event, Event::RelativeMode(_)) && !attempt.hid_mouse_enabled() {
            return Ok(());
        }
        let viewport = window.presented_viewport();
        let Some(packet) = self.frame(event, viewport) else {
            return Ok(());
        };
        if window.native_input_allowed()
            || matches!(event, Event::Focus(false) | Event::HidInputRelease)
        {
            attempt.native_input(&packet, viewport)?;
        }
        Ok(())
    }
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
    #[test]
    fn native_buttons_wheel_and_modifiers_match_lowlat_wire_values() {
        let mut input = Input::default();
        assert_eq!(
            input.frame(&Event::Button(true, 0, 0, 0), None).unwrap(),
            crate::control::header(1, 1, 1, 0)
        );
        assert_eq!(
            input.frame(&Event::Button(false, 2, 0, 0), None).unwrap(),
            crate::control::header(1, 3, 0, 0)
        );
        assert_eq!(
            input.frame(&Event::Scroll(120, -120), None).unwrap(),
            crate::control::header(2, 120, 120, 0)
        );
        assert_eq!(
            input
                .frame(&Event::Key(true, "ControlRight", 2), None)
                .unwrap(),
            crate::control::header(0, 228, 128, 1)
        );
        assert_eq!(
            input.frame(&Event::Key(true, "KeyA", 18), None).unwrap(),
            crate::control::header(0, 4, 128 | 8192, 1)
        );
        assert_eq!(
            input.frame(&Event::Focus(false), None).unwrap(),
            crate::control::header(24, 0, 0, 0)
        );
        assert_eq!(input.modifiers, 0);
    }
}
