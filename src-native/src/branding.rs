//! Shared application colors and embedded icons; no runtime assets or per-frame decoding.
use windows_sys::Win32::{System::LibraryLoader::*, UI::WindowsAndMessaging::*};

pub const BACKGROUND: u32 = 0x00231d19; // #191d23 as Win32 COLORREF
pub const BLUE: u32 = 0x00ed802f; // #2f80ed as Win32 COLORREF

pub unsafe fn icon() -> HICON {
    // LoadIcon returns a shared resource owned by Windows, not by the caller.
    LoadIconW(
        GetModuleHandleW(std::ptr::null()),
        std::ptr::without_provenance(1),
    )
}

pub unsafe fn launcher_icon() -> HICON {
    // Use the embedded 40 px resource; Windows owns this shared handle.
    LoadImageW(
        GetModuleHandleW(std::ptr::null()),
        std::ptr::without_provenance(1),
        IMAGE_ICON,
        40,
        40,
        LR_SHARED,
    ) as HICON
}
