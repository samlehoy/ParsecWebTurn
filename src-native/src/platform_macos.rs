//! Documented macOS Unicode clipboard, default browser and NSAlert APIs.
use crate::platform::{Desktop, MAX_TEXT};
use objc2_app_kit::{NSAlert, NSAlertStyle, NSPasteboard, NSPasteboardTypeString, NSWorkspace};
use objc2_foundation::{NSString, NSURL};

pub struct NativeDesktop(pub usize);

impl NativeDesktop {
    pub fn new(window_ptr: usize) -> Self {
        Self(window_ptr)
    }
}

impl Desktop for NativeDesktop {
    fn read_text(&self) -> Option<String> {
        let pasteboard = NSPasteboard::generalPasteboard();
        let ns_type = unsafe { NSPasteboardTypeString };
        let text = pasteboard.stringForType(ns_type)?;
        let s = text.to_string();
        if s.len() > MAX_TEXT {
            None
        } else {
            Some(s)
        }
    }

    fn write_text(&self, text: &str) -> bool {
        if text.len() > MAX_TEXT {
            return false;
        }
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        let ns_type = unsafe { NSPasteboardTypeString };
        let ns_str = NSString::from_str(text);
        pasteboard.setString_forType(&ns_str, ns_type)
    }

    fn open_url(&self, url: &str) -> bool {
        let trimmed = url.trim();
        if !trimmed.starts_with("https://") {
            return false;
        }
        let ns_str = NSString::from_str(trimmed);
        let Some(ns_url) = NSURL::URLWithString(&ns_str) else {
            return false;
        };
        let workspace = NSWorkspace::sharedWorkspace();
        workspace.openURL(&ns_url)
    }

    fn alert(&self, title: &str, message: &str) -> bool {
        let title = title.to_string();
        let message = message.to_string();
        dispatch2::DispatchQueue::main().exec_sync(move || {
            let mtm = unsafe { objc2::MainThreadMarker::new_unchecked() };
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&NSString::from_str(&title));
            alert.setInformativeText(&NSString::from_str(&message));
            alert.setAlertStyle(NSAlertStyle::Informational);
            alert.runModal();
        });
        true
    }
}
