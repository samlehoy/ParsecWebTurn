//! Optional, focus-scoped interception of shortcuts reserved by Windows.
//! No injection or administrator rights; Ctrl+Alt+Delete remains OS-owned.
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

static TARGET: AtomicUsize = AtomicUsize::new(0);
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
unsafe extern "system" fn callback(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let hwnd = TARGET.load(Ordering::Acquire) as HWND;
    if code == HC_ACTION as i32 && !hwnd.is_null() && GetForegroundWindow() == hwnd {
        let key = &*(lp as *const KBDLLHOOKSTRUCT);
        let vk = key.vkCode;
        let reserved = vk == VK_LWIN as u32
            || vk == VK_RWIN as u32
            || (key.flags & LLKHF_ALTDOWN != 0 && (vk == VK_TAB as u32 || vk == VK_ESCAPE as u32))
            || (vk == VK_ESCAPE as u32 && GetAsyncKeyState(VK_CONTROL as i32) < 0);
        if reserved {
            let down = matches!(wp as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            let packed = ((key.scanCode & 255) << 16)
                | if key.flags & LLKHF_EXTENDED != 0 {
                    1 << 24
                } else {
                    0
                }
                | if down { 0 } else { 1 << 31 };
            // The hook never calls WASM, waits or takes application locks.
            if PostMessageW(hwnd, WM_APP + 13, vk as usize, packed as isize) != 0 {
                return 1;
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wp, lp)
}
