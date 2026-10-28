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
// Physical locations of the shared client shortcut triggers. Translate through
// the current Windows layout; e.g. physical KeyQ need not have virtual key Q.
const SHORTCUT_SCANS: [u32; 7] = [0x10, 0x11, 0x13, 0x18, 0x1f, 0x32, 0xe04f];

/// Seed only state whose initial value changes modifier or shortcut behavior.
/// Other keys are tracked from foreground hook events; their initial repeat
/// flag cannot activate a client shortcut. No full-keyboard polling is needed.
fn initial_keys(
    mut down: impl FnMut(u16) -> bool,
    mut toggled: impl FnMut(u16) -> bool,
    mut virtual_key: impl FnMut(u32) -> u32,
) -> (usize, [bool; 256]) {
    let mut modifiers = 0;
    let mut pressed = [false; 256];
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
        pressed[usize::from(vk)] = down(vk);
        if pressed[usize::from(vk)] {
            modifiers |= modifier_bit(vk);
        }
    }
    for (vk, bit) in [(VK_CAPITAL, 256), (VK_NUMLOCK, 512)] {
        pressed[usize::from(vk)] = down(vk);
        if toggled(vk) {
            modifiers |= bit;
        }
    }
    for scan in SHORTCUT_SCANS {
        let vk = virtual_key(scan);
        if vk > 0 && vk < 256 {
            pressed[vk as usize] = down(vk as u16);
        }
    }
    (modifiers, pressed)
}
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
            let (keys, pressed) = initial_keys(
                |vk| GetAsyncKeyState(i32::from(vk)) < 0,
                |vk| GetKeyState(i32::from(vk)) & 1 != 0,
                |scan| MapVirtualKeyW(scan, MAPVK_VSC_TO_VK_EX),
            );
            MODIFIER_KEYS.store(keys, Ordering::Relaxed);
            for (key, down) in CAPTURED.iter().zip(pressed) {
                key.store(down, Ordering::Relaxed);
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
            } else {
                crate::event_log::record(
                    "WARN",
                    "immersive_keyboard_capture_failed",
                    serde_json::json!({"win32_error": GetLastError()}),
                );
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
        // Simulate a W that was already physically held when capture began.
        CAPTURED[b'W' as usize].store(true, Ordering::Relaxed);
        for (vk, scan, down, flags) in [
            (VK_LCONTROL, 0x1d, true, 0),
            (VK_LSHIFT, 0x2a, true, 0),
            (b'W' as u16, 0x11, true, 0),
            (b'W' as u16, 0x11, false, 0),
            (VK_LSHIFT, 0x2a, false, 0),
            (VK_LCONTROL, 0x1d, false, 0),
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
        // Synthetic modifiers must not depend on the runner's lock-key LEDs.
        MODIFIER_KEYS.store(0, Ordering::Relaxed);
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
    fn capture_seed_queries_only_required_keys_and_respects_layout_mapping() {
        let mut reads = Vec::new();
        let mut locks = Vec::new();
        let mut scans = Vec::new();
        let (keys, pressed) = initial_keys(
            |vk| {
                reads.push(vk);
                [VK_RCONTROL, VK_RSHIFT, VK_NUMLOCK, b'A' as u16, b'B' as u16].contains(&vk)
            },
            |vk| {
                locks.push(vk);
                vk == VK_CAPITAL
            },
            |scan| {
                scans.push(scan);
                // AZERTY-style virtual keys differ from physical shortcut names.
                match scan {
                    0x10 => u32::from(b'A'),
                    0x11 => u32::from(b'Z'),
                    0x13 => u32::from(b'R'),
                    0x18 => u32::from(b'O'),
                    0x1f => u32::from(b'S'),
                    0x32 => 0xbc,
                    0xe04f => u32::from(VK_END),
                    _ => panic!("unexpected physical key query"),
                }
            },
        );
        assert_eq!(reads.len(), 17);
        assert_eq!(locks, [VK_CAPITAL, VK_NUMLOCK]);
        assert_eq!(scans, SHORTCUT_SCANS);
        assert!(
            !reads.contains(&(b'B' as u16)),
            "ordinary keys must not be polled"
        );
        assert_eq!(pressed.iter().filter(|&&down| down).count(), 4);
        for vk in [VK_RCONTROL, VK_RSHIFT, VK_NUMLOCK, b'A' as u16] {
            assert!(pressed[usize::from(vk)]);
        }
        assert!(!pressed[usize::from(b'Q')]);
        assert_eq!(modifiers(keys), 1 | 2 | 16);
    }
    #[test]
    fn unmapped_shortcuts_are_not_queried_and_seed_targets_cover_shared_bindings() {
        let mut reads = Vec::new();
        let (keys, pressed) = initial_keys(
            |vk| {
                reads.push(vk);
                false
            },
            |_| false,
            |scan| if scan == 0x10 { 256 } else { 0 },
        );
        assert_eq!(reads.len(), 10);
        assert_eq!(keys, 0);
        assert!(!pressed.into_iter().any(|down| down));
        for scan in 1..=255u8 {
            if crate::input_windows::code(0, scan, false)
                .is_some_and(|code| crate::input::shortcut_binding(code).is_some())
            {
                assert!(SHORTCUT_SCANS.contains(&u32::from(scan)));
            }
        }
        assert!(SHORTCUT_SCANS.contains(&0xe04f));
        assert!(crate::input::shortcut_binding("End").is_some());
    }
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
