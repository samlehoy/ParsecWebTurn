//! Focus-scoped immersive keyboard forwarding on one ordered message path.
//! No injection or administrator rights; Ctrl+Alt+Delete remains OS-owned.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

static TARGET: AtomicUsize = AtomicUsize::new(0);
static GENERATION: AtomicUsize = AtomicUsize::new(0);
static MODIFIER_KEYS: AtomicUsize = AtomicUsize::new(0);
static CAPTURED: [AtomicBool; 256] = [const { AtomicBool::new(false) }; 256];
#[derive(Default)]
pub struct State {
    hook: usize,
}
impl State {
    pub unsafe fn set(&mut self, hwnd: HWND, enable: bool) {
        if !enable {
            TARGET.store(0, Ordering::Release);
            if self.hook != 0 {
                UnhookWindowsHookEx(self.hook as HHOOK);
                self.hook = 0;
            }
        } else if self.hook == 0 && GetForegroundWindow() == hwnd {
            let mut keys = 0;
            for vk in [
                VK_LSHIFT,
                VK_RSHIFT,
                VK_LCONTROL,
                VK_RCONTROL,
                VK_LMENU,
                VK_RMENU,
                VK_LWIN,
                VK_RWIN,
            ] {
                if GetAsyncKeyState(i32::from(vk)) < 0 {
                    keys |= modifier_bit(vk);
                }
            }
            for (vk, bit) in [(VK_CAPITAL, 256), (VK_NUMLOCK, 512)] {
                if GetKeyState(i32::from(vk)) & 1 != 0 {
                    keys |= bit;
                }
            }
            MODIFIER_KEYS.store(keys, Ordering::Relaxed);
            for key in &CAPTURED {
                key.store(false, Ordering::Relaxed);
            }
            let hook = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(callback),
                GetModuleHandleW(std::ptr::null()),
                0,
            );
            if !hook.is_null() {
                self.hook = hook as usize;
                GENERATION.fetch_add(1, Ordering::Relaxed);
                TARGET.store(hwnd as usize, Ordering::Release);
            }
        }
    }
    pub fn active(&self) -> bool {
        self.hook != 0
    }
}
pub fn current(message_key: WPARAM) -> bool {
    TARGET.load(Ordering::Acquire) != 0 && message_key >> 8 == GENERATION.load(Ordering::Relaxed)
}
fn modifier_bit(vk: u16) -> usize {
    match vk {
        VK_LSHIFT | VK_SHIFT => 1,
        VK_RSHIFT => 2,
        VK_LCONTROL | VK_CONTROL => 4,
        VK_RCONTROL => 8,
        VK_LMENU | VK_MENU => 16,
        VK_RMENU => 32,
        VK_LWIN => 64,
        VK_RWIN => 128,
        _ => 0,
    }
}
fn modifiers(keys: usize) -> usize {
    (0..4).fold((keys >> 4) & 48, |mods, group| {
        mods | (usize::from(keys & (3 << (group * 2)) != 0) << group)
    })
}
unsafe extern "system" fn callback(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let hwnd = TARGET.load(Ordering::Acquire) as HWND;
    if code == HC_ACTION as i32 && !hwnd.is_null() && GetForegroundWindow() == hwnd {
        let key = &*(lp as *const KBDLLHOOKSTRUCT);
        let vk = key.vkCode;
        let down = matches!(wp as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        let captured = CAPTURED.get(vk as usize);
        let repeated = down && captured.is_some_and(|key| key.load(Ordering::Relaxed));
        let mut keys = MODIFIER_KEYS.load(Ordering::Relaxed);
        let bit = modifier_bit(vk as u16);
        if bit != 0 {
            keys = if down { keys | bit } else { keys & !bit };
        }
        if down && !repeated {
            keys ^= match vk as u16 {
                VK_CAPITAL => 256,
                VK_NUMLOCK => 512,
                _ => 0,
            };
        }
        MODIFIER_KEYS.store(keys, Ordering::Relaxed);
        let mut mods = modifiers(keys);
        // LLKHF_ALTDOWN describes this event; a later UI-thread key-state
        // query could already see Alt released. Carry the event's modifiers.
        if key.flags & LLKHF_ALTDOWN != 0 && bit & 48 == 0 {
            mods |= 4;
        }
        // Capture modifiers and ordinary keys together. Mixing suppressed
        // hook messages with legacy WM_KEY messages can reorder a chord.
        if crate::input_windows::code(vk, key.scanCode as u8, key.flags & LLKHF_EXTENDED != 0)
            .is_some()
        {
            let packed = crate::input_windows::captured_key_data(key, down, mods)
                | if repeated { 1 << 30 } else { 0 };
            // The hook never waits or takes application locks.
            if PostMessageW(
                hwnd,
                crate::input_windows::CAPTURED_KEY,
                (GENERATION.load(Ordering::Relaxed) << 8) | vk as usize,
                packed,
            ) != 0
            {
                if let Some(key) = captured {
                    key.store(down, Ordering::Relaxed);
                }
                return 1;
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wp, lp)
}

#[cfg(test)]
pub(crate) fn test_immersive_chords(hwnd: HWND) -> WPARAM {
    unsafe {
        assert_eq!(TARGET.load(Ordering::Acquire), hwnd as usize);
        MODIFIER_KEYS.store(0, Ordering::Relaxed);
        for (vk, scan, down, flags) in [
            (VK_LMENU, 0x38, true, 0),
            (VK_RETURN, 0x1c, true, LLKHF_ALTDOWN),
            (VK_RETURN, 0x1c, false, LLKHF_ALTDOWN),
            (VK_LMENU, 0x38, false, LLKHF_ALTDOWN),
            (VK_LCONTROL, 0x1d, true, 0),
            (VK_RSHIFT, 0x36, true, 0),
            (b'W' as u16, 0x11, true, 0),
            (b'W' as u16, 0x11, true, 0),
            (VK_RSHIFT, 0x36, false, 0),
            (b'W' as u16, 0x11, false, 0),
            (VK_LCONTROL, 0x1d, false, 0),
            (VK_LCONTROL, 0x1d, true, 0),
            (VK_LSHIFT, 0x2a, true, 0),
            (VK_RMENU, 0x38, true, LLKHF_EXTENDED),
            (b'W' as u16, 0x11, true, LLKHF_ALTDOWN),
            (b'W' as u16, 0x11, false, LLKHF_ALTDOWN),
            (VK_RMENU, 0x38, false, LLKHF_EXTENDED | LLKHF_ALTDOWN),
            (VK_LSHIFT, 0x2a, false, 0),
            (VK_LCONTROL, 0x1d, false, 0),
            (VK_LWIN, 0x5b, true, LLKHF_EXTENDED),
            (b'R' as u16, 0x13, true, 0),
            (VK_LWIN, 0x5b, false, LLKHF_EXTENDED),
            (b'R' as u16, 0x13, false, 0),
            (VK_F8, 0x42, true, 0),
            (VK_F8, 0x42, false, 0),
            (VK_F11, 0x57, true, 0),
            (VK_F11, 0x57, false, 0),
            (VK_LCONTROL, 0x1d, true, 0),
            (VK_OEM_COMMA, 0x33, true, 0),
            (VK_OEM_COMMA, 0x33, false, 0),
            (VK_LCONTROL, 0x1d, false, 0),
            (VK_F12, 0x58, true, 0),
            (VK_F12, 0x58, false, 0),
        ] {
            let key = KBDLLHOOKSTRUCT {
                vkCode: u32::from(vk),
                scanCode: scan,
                flags: flags | if down { 0 } else { LLKHF_UP },
                ..std::mem::zeroed()
            };
            assert_eq!(
                callback(
                    HC_ACTION as i32,
                    if down { WM_KEYDOWN } else { WM_KEYUP } as usize,
                    (&key as *const KBDLLHOOKSTRUCT) as isize
                ),
                1
            );
        }
        assert_eq!(MODIFIER_KEYS.load(Ordering::Relaxed), 0);
        (GENERATION.load(Ordering::Relaxed) << 8) | usize::from(VK_F12)
    }
}

#[cfg(test)]
pub(crate) fn test_reserved_pair(hwnd: HWND) {
    unsafe {
        assert_eq!(TARGET.load(Ordering::Acquire), hwnd as usize);
        let down = KBDLLHOOKSTRUCT {
            vkCode: VK_TAB as u32,
            scanCode: 15,
            flags: LLKHF_ALTDOWN,
            ..std::mem::zeroed()
        };
        assert_eq!(
            callback(
                HC_ACTION as i32,
                WM_SYSKEYDOWN as usize,
                (&down as *const KBDLLHOOKSTRUCT) as isize
            ),
            1
        );
        let up = KBDLLHOOKSTRUCT {
            flags: LLKHF_UP,
            ..down
        };
        assert_eq!(
            callback(
                HC_ACTION as i32,
                WM_KEYUP as usize,
                (&up as *const KBDLLHOOKSTRUCT) as isize
            ),
            1
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modifier_sides_and_lock_bits_are_independent() {
        let keys = modifier_bit(VK_LCONTROL) | modifier_bit(VK_RCONTROL);
        assert_eq!(modifiers(keys & !modifier_bit(VK_LCONTROL)), 2);
        assert_eq!(modifiers(255), 15);
        assert_eq!(modifiers(256), 16);
        assert_eq!(modifiers(512), 32);
        assert_eq!(modifiers(1023), 63);
    }
}
