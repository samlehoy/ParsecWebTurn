//! Optional, focus-scoped interception of shortcuts reserved by Windows.
//! No injection or administrator rights; Ctrl+Alt+Delete remains OS-owned.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

static TARGET: AtomicUsize = AtomicUsize::new(0);
static MODIFIERS: AtomicUsize = AtomicUsize::new(0);
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
            let mut mods = 0;
            for (keys, bit) in [
                ([VK_LSHIFT, VK_RSHIFT], 1),
                ([VK_LCONTROL, VK_RCONTROL], 2),
                ([VK_LMENU, VK_RMENU], 4),
                ([VK_LWIN, VK_RWIN], 8),
            ] {
                if keys.iter().any(|vk| GetAsyncKeyState(i32::from(*vk)) < 0) {
                    mods |= bit;
                }
            }
            MODIFIERS.store(mods, Ordering::Relaxed);
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
                TARGET.store(hwnd as usize, Ordering::Release);
            }
        }
    }
    pub fn active(&self) -> bool {
        self.hook != 0
    }
}
fn reserved(vk: u32, mods: usize) -> bool {
    vk == VK_LWIN as u32
        || vk == VK_RWIN as u32
        || (mods & 4 != 0 && (vk == VK_TAB as u32 || vk == VK_ESCAPE as u32 || vk == VK_F4 as u32))
        || (vk == VK_ESCAPE as u32 && mods & 2 != 0)
}
unsafe extern "system" fn callback(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let hwnd = TARGET.load(Ordering::Acquire) as HWND;
    if code == HC_ACTION as i32 && !hwnd.is_null() && GetForegroundWindow() == hwnd {
        let key = &*(lp as *const KBDLLHOOKSTRUCT);
        let vk = key.vkCode;
        let down = matches!(wp as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        let mut mods = MODIFIERS.load(Ordering::Relaxed);
        let bit = match vk as u16 {
            VK_LSHIFT | VK_RSHIFT | VK_SHIFT => 1,
            VK_LCONTROL | VK_RCONTROL | VK_CONTROL => 2,
            VK_LMENU | VK_RMENU | VK_MENU => 4,
            VK_LWIN | VK_RWIN => 8,
            _ => 0,
        };
        if bit != 0 {
            mods = if down { mods | bit } else { mods & !bit };
        }
        MODIFIERS.store(mods, Ordering::Relaxed);
        // LLKHF_ALTDOWN describes this event; a later UI-thread key-state
        // query could already see Alt released. Carry the event's modifiers.
        if key.flags & LLKHF_ALTDOWN != 0 {
            mods |= 4;
        }
        let captured = CAPTURED.get(vk as usize);
        if reserved(vk, mods) || captured.is_some_and(|key| key.load(Ordering::Relaxed)) {
            let packed = ((key.scanCode & 255) << 16)
                | mods as u32
                | if key.flags & LLKHF_EXTENDED != 0 {
                    1 << 24
                } else {
                    0
                }
                | if down { 0 } else { 1 << 31 };
            // The hook never calls WASM, waits or takes application locks.
            if PostMessageW(hwnd, WM_APP + 13, vk as usize, packed as isize) != 0 {
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
    fn windows_switcher_close_and_task_manager_shortcuts_are_remote_only_while_grabbed() {
        assert!(reserved(VK_LWIN as u32, 0));
        assert!(reserved(VK_TAB as u32, 4));
        assert!(reserved(VK_F4 as u32, 4));
        assert!(reserved(VK_ESCAPE as u32, 3));
        assert!(!reserved(VK_TAB as u32, 0));
        assert!(!reserved(VK_F4 as u32, 0));
        // The core's own immersive toggle remains on its normal key path.
        assert!(!reserved(u32::from(b'I'), 3));
    }
}
