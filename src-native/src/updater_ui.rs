//! Native modeless update window. Network workers never access HWNDs.
use crate::updater::{Phase, State, View};
use std::{
    ptr::{null, null_mut},
    sync::Arc,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::*,
    UI::{
        Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS},
        WindowsAndMessaging::*,
    },
};
pub const OPEN: usize = 4302;
pub const CLOSED: u32 = WM_APP + 15;
pub const START: u32 = WM_APP + 16;
pub const NOTIFY_TIMER: usize = 4302;
const CHECK: usize = 4400;
const DOWNLOAD: usize = 4401;
const CLOSE: usize = 4402;
const STATUS: usize = 4403;
const NOTES: usize = 4404;
const VERSION: usize = 4405;
const BG: u32 = 0x282828;
const FIELD: u32 = 0x212121;
const FG: u32 = 0xeeeeee;
const CYAN: u32 = 0xffbb00;
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
struct Page {
    owned: bool,
    state: Arc<State>,
    last: Option<View>,
    background: HBRUSH,
    field: HBRUSH,
    font: HFONT,
    heading: HFONT,
}
impl Drop for Page {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.background);
            DeleteObject(self.field);
            DeleteObject(self.font);
            DeleteObject(self.heading);
        }
    }
}
unsafe fn font(size: i32) -> HFONT {
    CreateFontW(
        -size,
        0,
        0,
        0,
        400,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        0,
        0,
        CLEARTYPE_QUALITY as u32,
        0,
        wide("Segoe UI").as_ptr(),
    )
}
unsafe fn add(parent: HWND, class: &str, title: &str, id: usize, style: u32, font: HFONT) -> HWND {
    let hwnd = CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(title).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        0,
        0,
        1,
        1,
        parent,
        id as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    SendMessageW(hwnd, WM_SETFONT, font as usize, 0);
    hwnd
}
pub unsafe fn open(parent: HWND, state: Arc<State>, activate: bool) -> HWND {
    let class = wide("ParsecWebTurnUpdates");
    let instance = GetModuleHandleW(null());
    RegisterClassW(&WNDCLASSW {
        lpfnWndProc: Some(proc),
        hInstance: instance,
        hIcon: LoadIconW(instance, std::ptr::without_provenance(1)),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        lpszClassName: class.as_ptr(),
        ..std::mem::zeroed()
    });
    let mut page = Box::new(Page {
        owned: false,
        state,
        last: None,
        background: CreateSolidBrush(BG),
        field: CreateSolidBrush(FIELD),
        font: font(16),
        heading: font(32),
    });
    let ptr = (&mut *page) as *mut Page;
    let hwnd = CreateWindowExW(
        WS_EX_CONTROLPARENT,
        class.as_ptr(),
        wide("ParsecWebTurn - Updates").as_ptr(),
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        740,
        680,
        parent,
        null_mut(),
        instance,
        ptr.cast(),
    );
    if hwnd.is_null() {
        return hwnd;
    }
    page.owned = true;
    let page = &mut *Box::into_raw(page);
    add(hwnd, "STATIC", "Updates", 1, 0, page.heading);
    add(hwnd, "STATIC", "", VERSION, 0, page.font);
    add(hwnd, "STATIC", "", STATUS, 0, page.font);
    let notes = add(
        hwnd,
        "EDIT",
        "",
        NOTES,
        ES_MULTILINE as u32
            | ES_READONLY as u32
            | ES_AUTOVSCROLL as u32
            | WS_VSCROLL
            | WS_TABSTOP
            | WS_CLIPCHILDREN,
        page.font,
    );
    crate::connection_settings_ui::controls::multiline(notes);
    for (id, label) in [
        (CHECK, "Check again"),
        (DOWNLOAD, "Download ZIP"),
        (CLOSE, "Close"),
    ] {
        add(
            hwnd,
            "BUTTON",
            label,
            id,
            BS_OWNERDRAW as u32 | WS_TABSTOP,
            page.font,
        );
    }
    refresh(hwnd, page);
    layout(hwnd);
    SetTimer(hwnd, 1, 500, None);
    ShowWindow(hwnd, if activate { SW_SHOW } else { SW_SHOWNOACTIVATE });
    if activate {
        SetFocus(GetDlgItem(hwnd, CHECK as i32));
    }
    hwnd
}
unsafe fn layout(hwnd: HWND) {
    let mut rect: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    let width = (rect.right - 48).max(1);
    let bottom = rect.bottom - 62;
    for (id, x, y, w, h) in [
        (1, 24, 20, width, 48),
        (VERSION, 24, 76, width, 42),
        (STATUS, 24, 126, width, 78),
        (NOTES, 24, 214, width, (bottom - 234).max(1)),
        (CHECK, 24, bottom, 142, 36),
        (DOWNLOAD, 178, bottom, 150, 36),
        (CLOSE, rect.right - 132, bottom, 108, 36),
    ] {
        SetWindowPos(
            GetDlgItem(hwnd, id as i32),
            null_mut(),
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
}
unsafe fn refresh(hwnd: HWND, page: &mut Page) {
    let view = page.state.view();
    if page.last.as_ref() == Some(&view) {
        return;
    }
    let version = format!(
        "Installed: v{}{}",
        env!("CARGO_PKG_VERSION"),
        view.release
            .as_ref()
            .map(|r| format!("\r\nAvailable: {} (stable)", r.version))
            .unwrap_or_default()
    );
    SetWindowTextW(GetDlgItem(hwnd, VERSION as i32), wide(&version).as_ptr());
    SetWindowTextW(GetDlgItem(hwnd, STATUS as i32), wide(view.message).as_ptr());
    let notes = view
        .release
        .as_ref()
        .map(|r| r.notes.replace("\r\n", "\n").replace('\n', "\r\n"))
        .unwrap_or_default();
    SetWindowTextW(GetDlgItem(hwnd, NOTES as i32), wide(&notes).as_ptr());
    EnableWindow(
        GetDlgItem(hwnd, CHECK as i32),
        i32::from(view.phase != Phase::Checking),
    );
    ShowWindow(
        GetDlgItem(hwnd, DOWNLOAD as i32),
        if view.phase == Phase::Available {
            SW_SHOW
        } else {
            SW_HIDE
        },
    );
    page.last = Some(view);
    InvalidateRect(hwnd, null(), 1);
}
unsafe fn draw_button(draw: &DRAWITEMSTRUCT, page: &Page) {
    let saved = SaveDC(draw.hDC);
    let active = draw.CtlID == DOWNLOAD as u32 && draw.itemState & ODS_DISABLED == 0;
    let brush = CreateSolidBrush(if active { CYAN } else { FIELD });
    FillRect(draw.hDC, &draw.rcItem, brush);
    DeleteObject(brush);
    SelectObject(draw.hDC, page.font);
    SetBkMode(draw.hDC, TRANSPARENT as i32);
    SetTextColor(
        draw.hDC,
        if draw.itemState & ODS_DISABLED != 0 {
            0x777777
        } else if active {
            BG
        } else {
            FG
        },
    );
    let mut text = [0u16; 64];
    GetWindowTextW(draw.hwndItem, text.as_mut_ptr(), 64);
    let mut rect = draw.rcItem;
    DrawTextW(
        draw.hDC,
        text.as_ptr(),
        -1,
        &mut rect,
        DT_SINGLELINE | DT_VCENTER | DT_CENTER,
    );
    if draw.itemState & ODS_FOCUS != 0 {
        InflateRect(&mut rect, -3, -3);
        DrawFocusRect(draw.hDC, &rect);
    }
    RestoreDC(draw.hDC, saved);
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
        );
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Page;
    if !ptr.is_null() {
        let page = &mut *ptr;
        match msg {
            WM_TIMER => {
                refresh(hwnd, page);
                return 0;
            }
            WM_SIZE => {
                layout(hwnd);
                return 0;
            }
            WM_GETMINMAXINFO => {
                let info = &mut *(lp as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT { x: 560, y: 500 };
                return 0;
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
                let dc = wp as HDC;
                SetTextColor(dc, FG);
                let notes = lp as HWND == GetDlgItem(hwnd, NOTES as i32);
                SetBkColor(dc, if notes { FIELD } else { BG });
                return if notes { page.field } else { page.background } as isize;
            }
            WM_ERASEBKGND => {
                let mut rect = std::mem::zeroed();
                GetClientRect(hwnd, &mut rect);
                FillRect(wp as HDC, &rect, page.background);
                return 1;
            }
            WM_DRAWITEM => {
                draw_button(&*(lp as *const DRAWITEMSTRUCT), page);
                return 1;
            }
            WM_COMMAND => match wp & 0xffff {
                CHECK => {
                    page.state.check(false);
                    refresh(hwnd, page);
                    return 0;
                }
                DOWNLOAD => {
                    if let Some(release) = page.state.view().release {
                        use crate::platform::Desktop;
                        if !crate::platform_windows::NativeDesktop(hwnd as usize)
                            .open_url(&release.download_url)
                        {
                            MessageBoxW(hwnd, wide("Cannot open the download in your browser. Your organization may restrict it.").as_ptr(),
                                wide("Updates").as_ptr(), MB_OK | MB_ICONINFORMATION);
                        }
                    }
                    return 0;
                }
                CLOSE | 2 => {
                    DestroyWindow(hwnd);
                    return 0;
                }
                _ => {}
            },
            WM_NCDESTROY => {
                KillTimer(hwnd, 1);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                SendMessageW(GetWindow(hwnd, GW_OWNER), CLOSED, hwnd as usize, 0);
                if page.owned {
                    drop(Box::from_raw(ptr));
                }
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modeless_updates_resize_and_close_without_network_or_browser() {
        unsafe {
            let parent = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("Isolated update fixture").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                800,
                800,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!parent.is_null());
            let state = Arc::new(State::default());
            let hwnd = open(parent, state, false);
            assert!(!hwnd.is_null());
            assert_eq!(IsWindowEnabled(GetDlgItem(hwnd, CHECK as i32)), 1);
            assert_eq!(IsWindowVisible(GetDlgItem(hwnd, DOWNLOAD as i32)), 0);
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                560,
                500,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let mut notes = RECT::default();
            let mut close = RECT::default();
            GetWindowRect(GetDlgItem(hwnd, NOTES as i32), &mut notes);
            GetWindowRect(GetDlgItem(hwnd, CLOSE as i32), &mut close);
            assert!(
                notes.bottom < close.top && notes.right > notes.left && notes.bottom > notes.top
            );
            SendMessageW(hwnd, WM_COMMAND, CLOSE, 0);
            assert_eq!(IsWindow(hwnd), 0);
            DestroyWindow(parent);
        }
    }
}
