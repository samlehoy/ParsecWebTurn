//! One embedded application icon; no runtime assets or per-frame icon decoding.
use windows_sys::Win32::{Foundation::*, System::LibraryLoader::*, UI::WindowsAndMessaging::*};

pub unsafe fn icon() -> HICON {
    // LoadIcon returns a shared resource owned by Windows, not by the caller.
    LoadIconW(
        GetModuleHandleW(std::ptr::null()),
        std::ptr::without_provenance(1),
    )
}

#[cfg(feature = "native-core")]
pub unsafe fn about(owner: HWND) {
    let text: Vec<u16> = format!("ParsecWebTurn {}\nIndependent native Rust client — BUD transport.\nNo Parsec WASM core is loaded or executed.",env!("CARGO_PKG_VERSION")).encode_utf16().chain([0]).collect();
    let caption: Vec<u16> = "About ParsecWebTurn".encode_utf16().chain([0]).collect();
    MessageBoxIndirectW(&MSGBOXPARAMSW {
        cbSize: std::mem::size_of::<MSGBOXPARAMSW>() as u32,
        hwndOwner: owner,
        hInstance: GetModuleHandleW(std::ptr::null()),
        lpszText: text.as_ptr(),
        lpszCaption: caption.as_ptr(),
        dwStyle: MB_OK | MB_USERICON,
        lpszIcon: std::ptr::without_provenance(1),
        ..std::mem::zeroed()
    });
}
