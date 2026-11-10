//! Documented Win32 Unicode clipboard ownership and text APIs.
use crate::platform::{Desktop, MAX_TEXT};
use windows_sys::Win32::{
    Foundation::*,
    System::{DataExchange::*, Memory::*},
    UI::WindowsAndMessaging::*,
};

pub struct NativeDesktop(pub usize);
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
struct Clipboard;
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}
struct Locked(HGLOBAL);
impl Drop for Locked {
    fn drop(&mut self) {
        unsafe {
            GlobalUnlock(self.0);
        }
    }
}
impl NativeDesktop {
    fn owner(&self) -> HWND {
        self.0 as HWND
    }
    fn clipboard(&self) -> Option<Clipboard> {
        // Match the focused local UI use case. Never read the system clipboard
        // from an unfocused background session or from offline probes.
        unsafe {
            if GetForegroundWindow() == self.owner() && OpenClipboard(self.owner()) != 0 {
                Some(Clipboard)
            } else {
                None
            }
        }
    }
}

impl Desktop for NativeDesktop {
    fn read_text(&self) -> Option<String> {
        let _clipboard = self.clipboard()?;
        unsafe {
            let handle = GetClipboardData(13); // CF_UNICODETEXT
            if handle.is_null() {
                return None;
            }
            let size = GlobalSize(handle);
            if !(2..=(MAX_TEXT + 1) * 2).contains(&size) || !size.is_multiple_of(2) {
                return None;
            }
            let ptr = GlobalLock(handle).cast::<u16>();
            if ptr.is_null() {
                return None;
            }
            let _locked = Locked(handle);
            // The clipboard is open, the handle locked, and its size bounded.
            let chars = std::slice::from_raw_parts(ptr, size / 2);
            let end = chars.iter().position(|c| *c == 0)?;
            String::from_utf16(&chars[..end]).ok()
        }
    }
    fn write_text(&self, text: &str) -> bool {
        let Some(_clipboard) = self.clipboard() else {
            return false;
        };
        let chars = wide(text);
        unsafe {
            let handle = GlobalAlloc(GMEM_MOVEABLE, chars.len() * 2);
            if handle.is_null() {
                return false;
            }
            let ptr = GlobalLock(handle).cast::<u16>();
            if ptr.is_null() {
                GlobalFree(handle);
                return false;
            }
            std::ptr::copy_nonoverlapping(chars.as_ptr(), ptr, chars.len());
            GlobalUnlock(handle);
            if EmptyClipboard() == 0 || SetClipboardData(13, handle).is_null() {
                GlobalFree(handle);
                return false;
            }
            // SetClipboardData transfers ownership to Windows on success.
            true
        }
    }
}
