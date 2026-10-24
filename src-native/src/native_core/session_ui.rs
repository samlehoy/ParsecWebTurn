//! Event-driven native session panel. GDI controls overlay the video child;
//! no video pixels, timer, render loop or additional decoded-frame copy.
use super::{
    profile::{Resolution, Video},
    ui::{draw_button, wide},
};
use anyhow::{ensure, Result};
use std::{
    ptr::{null, null_mut},
    sync::{atomic::Ordering, Arc},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::*,
    UI::{Controls::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
pub const OPEN: usize = 4600;
const APPLY: usize = 4602;
const CLOSE: usize = 4603;
const DISCONNECT: usize = 4604;
const SETTINGS: usize = 4605;
const STATUS: i32 = 4606;
pub const RESULT: u32 = WM_APP + 26;
pub const VISIBILITY: u32 = WM_APP + 27;
const BG: u32 = 0x00231d19;
const FIELD: u32 = 0x003a322c;
const FG: u32 = 0x00f5f3f0;
#[derive(Clone, Debug)]
pub enum Action {
    Apply(Video),
    Disconnect,
}
struct Panel {
    window: *const crate::window::Window,
    window_owned: bool,
    scroll: i32,
    controls: [HWND; 6],
    fullscreen: HWND,
    font: HFONT,
    heading: HFONT,
    bg: HBRUSH,
    field: HBRUSH,
}
impl Drop for Panel {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.font);
            DeleteObject(self.heading);
            DeleteObject(self.bg);
            DeleteObject(self.field);
        }
    }
}
unsafe fn text(h: HWND) -> String {
    let n = GetWindowTextLengthW(h).clamp(0, 256);
    let mut b = vec![0; n as usize + 1];
    let used = GetWindowTextW(h, b.as_mut_ptr(), b.len() as i32).max(0) as usize;
    String::from_utf16_lossy(&b[..used])
}
fn number(text: &str, max: u32) -> Result<Option<u32>> {
    if text == "Keep host" {
        return Ok(None);
    }
    let n: u32 = text
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Enter a number, or select Keep host."))?;
    ensure!((1..=max).contains(&n), "Value must be 1–{max}.");
    Ok(Some(n))
}
fn resolution(text: &str) -> Result<Resolution> {
    match text.trim() {
        "Keep host" => Ok(Resolution::Host),
        "Match client display" => Ok(Resolution::Client),
        s => {
            let (w, h) = s
                .split_once(['x', '×'])
                .ok_or_else(|| anyhow::anyhow!("Enter resolution as width x height."))?;
            Ok(Resolution::Fixed(w.trim().parse()?, h.trim().parse()?))
        }
    }
}
unsafe fn collect(p: &Panel) -> Result<Video> {
    let select = |i: usize| SendMessageW(p.controls[i], CB_GETCURSEL, 0, 0);
    let v = Video {
        resolution: resolution(&text(p.controls[0]))?,
        bandwidth_mbps: number(&text(p.controls[1]), 100)?,
        fps: number(&text(p.controls[2]), 240)?,
        constant_fps: match select(3) {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        },
        rotated: match select(4) {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        },
        immersive: match select(5) {
            1 => 3,
            2 => 2,
            3 => 1,
            _ => 0,
        },
        fullscreen: SendMessageW(p.fullscreen, BM_GETCHECK, 0, 0) == BST_CHECKED as isize,
    };
    v.validate()?;
    Ok(v)
}
unsafe fn close(hwnd: HWND) {
    let parent = GetParent(hwnd);
    DestroyWindow(hwnd);
    SetFocus(parent);
}
unsafe fn scroll(hwnd: HWND, p: &mut Panel, target: i32) {
    if target == p.scroll {
        return;
    }
    let delta = p.scroll - target;
    p.scroll = target;
    ScrollWindowEx(
        hwnd,
        0,
        delta,
        null(),
        null(),
        null_mut(),
        null_mut(),
        SW_SCROLLCHILDREN | SW_INVALIDATE | SW_ERASE,
    );
    SetScrollPos(hwnd, SB_VERT, target, 1);
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
        );
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Panel;
    if !pointer.is_null() {
        let p = &mut *pointer;
        match msg {
            WM_ACTIVATE => {
                PostMessageW(GetParent(hwnd), VISIBILITY, 0, 0);
            }
            WM_SIZE => {
                let mut r = std::mem::zeroed();
                GetClientRect(hwnd, &mut r);
                let max = (580 - r.bottom).max(0);
                let target = p.scroll.min(max);
                scroll(hwnd, p, target);
                let info = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                    nMin: 0,
                    nMax: 579,
                    nPage: r.bottom.max(1) as u32,
                    nPos: p.scroll,
                    ..std::mem::zeroed()
                };
                SetScrollInfo(hwnd, SB_VERT, &info, 1);
                return 0;
            }
            WM_VSCROLL => {
                let mut info = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_ALL,
                    ..std::mem::zeroed()
                };
                GetScrollInfo(hwnd, SB_VERT, &mut info);
                let target = match wp & 0xffff {
                    x if x == SB_LINEUP as usize => p.scroll - 28,
                    x if x == SB_LINEDOWN as usize => p.scroll + 28,
                    x if x == SB_PAGEUP as usize => p.scroll - info.nPage as i32,
                    x if x == SB_PAGEDOWN as usize => p.scroll + info.nPage as i32,
                    x if x == SB_THUMBTRACK as usize => info.nTrackPos,
                    _ => p.scroll,
                };
                scroll(hwnd, p, target.clamp(0, (580 - info.nPage as i32).max(0)));
                return 0;
            }

            WM_DRAWITEM => {
                if draw_button(lp) {
                    return 1;
                }
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
                SetTextColor(wp as HDC, FG);
                SetBkColor(wp as HDC, if msg == WM_CTLCOLORSTATIC { BG } else { FIELD });
                return if msg == WM_CTLCOLORSTATIC {
                    p.bg
                } else {
                    p.field
                } as isize;
            }
            WM_ERASEBKGND => {
                let mut r = std::mem::zeroed();
                GetClientRect(hwnd, &mut r);
                FillRect(wp as HDC, &r, p.bg);
                return 1;
            }
            RESULT => {
                EnableWindow(GetDlgItem(hwnd, APPLY as i32), 1);
                SetWindowTextW(
                    GetDlgItem(hwnd, STATUS),
                    wide(&if wp == 2 {
                        let observed = (&*p.window)
                            .native_host_video
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        format!(
                            "Host: {} x {} | {} FPS\n{} Mbps | Constant FPS: {}",
                            observed["width"],
                            observed["height"],
                            observed["fps"],
                            observed["bandwidth_mbps"],
                            observed["constant_fps"]
                        )
                    } else if wp == 0 {
                        "Applied locally; video settings requested from host.".into()
                    } else {
                        "Could not apply or save settings. See the event log.".into()
                    })
                    .as_ptr(),
                );
                return 0;
            }
            WM_COMMAND => {
                match wp & 0xffff {
                    APPLY => match collect(p) {
                        Ok(video) => {
                            EnableWindow(GetDlgItem(hwnd, APPLY as i32), 0);
                            (&*p.window)
                                .events
                                .push(crate::window::Event::NativeSession(Action::Apply(video)));
                            SetWindowTextW(GetDlgItem(hwnd, STATUS), wide("Applying...").as_ptr());
                        }
                        Err(e) => super::ui::message_box(hwnd, &e.to_string()),
                    },
                    CLOSE | 2 => {
                        close(hwnd);
                    }
                    DISCONNECT => {
                        (&*p.window)
                            .events
                            .push(crate::window::Event::NativeSession(Action::Disconnect));
                    }
                    SETTINGS => {
                        let parent = GetParent(hwnd);
                        close(hwnd);
                        PostMessageW(parent, WM_COMMAND, crate::connection_settings_ui::OPEN, 0);
                    }
                    _ => {}
                }
                return 0;
            }
            WM_NCDESTROY => {
                (&*p.window).native_panel.store(0, Ordering::Release);
                #[cfg(test)]
                RemovePropW(hwnd, wide("ParsecWebTurn.SessionPanel.Ready").as_ptr());
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                if p.window_owned {
                    drop(Box::from_raw(pointer));
                }
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
unsafe fn control(
    hwnd: HWND,
    p: &Panel,
    class: &str,
    label: &str,
    id: usize,
    style: u32,
    r: [i32; 4],
) -> HWND {
    let h = CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(label).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        r[0],
        r[1],
        r[2],
        r[3],
        hwnd,
        id as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    SendMessageW(h, WM_SETFONT, p.font as usize, 0);
    h
}
unsafe fn combo(
    hwnd: HWND,
    p: &Panel,
    id: usize,
    y: i32,
    items: &[&str],
    value: &str,
    editable: bool,
) -> HWND {
    let h = control(
        hwnd,
        p,
        "COMBOBOX",
        "",
        id,
        WS_TABSTOP
            | WS_VSCROLL
            | if editable {
                CBS_DROPDOWN as u32
            } else {
                CBS_DROPDOWNLIST as u32
            },
        [174, y, 218, 260],
    );
    crate::connection_settings_ui::controls::combo(h);
    for item in items {
        SendMessageW(h, CB_ADDSTRING, 0, wide(item).as_ptr() as isize);
    }
    if editable {
        SetWindowTextW(h, wide(value).as_ptr());
        SendMessageW(h, CB_LIMITTEXT, 64, 0);
    } else {
        let n = items.iter().position(|s| *s == value).unwrap_or(0);
        SendMessageW(h, CB_SETCURSEL, n, 0);
    }
    h
}
unsafe fn panel(window: &crate::window::Window) -> HWND {
    window.native_panel.load(Ordering::Acquire) as HWND
}
unsafe fn panel_for_parent(parent: HWND) -> HWND {
    let w = GetWindowLongPtrW(parent, GWLP_USERDATA) as *const crate::window::Window;
    if w.is_null() {
        null_mut()
    } else {
        panel(&*w)
    }
}
// A standard child BUTTON sends owner-draw notifications to its parent.
// Keep its small popup container above the video, rather than making the
// BUTTON itself a top-level window (which has no child notification parent).
unsafe extern "system" fn badge_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let owner = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as HWND;
    match msg {
        WM_COMMAND if wp & 65535 == OPEN && (wp >> 16) as u32 == BN_CLICKED => {
            SendMessageW(owner, WM_COMMAND, OPEN, 0);
            return 0;
        }
        BM_CLICK => {
            SendMessageW(GetDlgItem(hwnd, OPEN as i32), BM_CLICK, 0, 0);
            return 0;
        }
        WM_DRAWITEM if draw_button(lp) => return 1,
        WM_ERASEBKGND => return 1,
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
pub unsafe fn install(window: &crate::window::Window) {
    let parent = window.handle();
    if window.native_session_button.load(Ordering::Acquire) == 0 {
        let class = wide("ParsecWebTurn.NativeSessionBadge");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(badge_proc),
            hInstance: GetModuleHandleW(null()),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        });
        let button = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            class.as_ptr(),
            wide("Session").as_ptr(),
            WS_POPUP | WS_CLIPCHILDREN,
            16,
            16,
            108,
            34,
            parent,
            null_mut(),
            GetModuleHandleW(null()),
            parent.cast(),
        );
        if !button.is_null() {
            let child = CreateWindowExW(
                0,
                wide("BUTTON").as_ptr(),
                wide("Session").as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
                0,
                0,
                108,
                34,
                button,
                OPEN as HMENU,
                GetModuleHandleW(null()),
                null(),
            );
            if child.is_null() {
                DestroyWindow(button);
                return;
            }
            SendMessageW(
                child,
                WM_SETFONT,
                GetStockObject(DEFAULT_GUI_FONT) as usize,
                1,
            );
            SetLayeredWindowAttributes(button, 0, 245, LWA_ALPHA);
            window
                .native_session_button
                .store(button as usize, Ordering::Release);
        }
    }
    raise(window);
}
pub unsafe fn raise(window: &crate::window::Window) {
    let parent = window.handle();
    let foreground = GetForegroundWindow();
    let active = foreground == parent
        || (!foreground.is_null() && GetAncestor(foreground, GA_ROOTOWNER) == parent);
    let visible = active
        && !window.settings_open()
        && !window.closing.load(Ordering::Acquire)
        && IsIconic(parent) == 0
        && IsWindowVisible(parent) != 0;
    let mut origin = POINT { x: 16, y: 16 };
    ClientToScreen(parent, &mut origin);
    let button = window.native_session_button.load(Ordering::Acquire) as HWND;
    if !button.is_null() {
        SetWindowPos(
            button,
            HWND_TOP,
            origin.x,
            origin.y,
            108,
            34,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
        ShowWindow(button, if visible { SW_SHOWNA } else { SW_HIDE });
    }
    let panel = panel(window);
    if !panel.is_null() {
        let mut r = std::mem::zeroed();
        GetClientRect(parent, &mut r);
        SetWindowPos(
            panel,
            HWND_TOP,
            origin.x,
            origin.y + 44,
            420,
            (r.bottom - 76).clamp(120, 580),
            SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
        ShowWindow(panel, if visible { SW_SHOWNA } else { SW_HIDE });
    }
}
pub unsafe fn toggle(
    window: &crate::window::Window,
    manager: Arc<crate::connection_settings::Manager>,
) {
    let parent = window.handle();
    let existing = panel_for_parent(parent);
    if !existing.is_null() {
        DestroyWindow(existing);
        SetFocus(parent);
        return;
    }
    let video = manager.view().0.native_core.video;
    let mut panel = Box::new(Panel {
        window,
        window_owned: false,
        scroll: 0,
        controls: [null_mut(); 6],
        fullscreen: null_mut(),
        font: super::ui::font(16, 400),
        heading: super::ui::font(25, 600),
        bg: CreateSolidBrush(BG),
        field: CreateSolidBrush(FIELD),
    });
    let name = wide("ParsecWebTurn.NativeCore.SessionPanel");
    let instance = GetModuleHandleW(null());
    RegisterClassW(&WNDCLASSW {
        lpfnWndProc: Some(proc),
        hInstance: instance,
        lpszClassName: name.as_ptr(),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        ..std::mem::zeroed()
    });
    let pointer = &mut *panel as *mut Panel;
    let hwnd = CreateWindowExW(
        WS_EX_CONTROLPARENT | WS_EX_LAYERED | WS_EX_TOOLWINDOW,
        name.as_ptr(),
        wide("Session controls").as_ptr(),
        WS_POPUP | WS_CLIPCHILDREN | WS_VSCROLL,
        16,
        60,
        420,
        580,
        parent,
        null_mut(),
        instance,
        pointer.cast(),
    );
    if hwnd.is_null() {
        return;
    }
    panel.window_owned = true;
    window.native_panel.store(hwnd as usize, Ordering::Release);
    SetLayeredWindowAttributes(hwnd, 0, 238, LWA_ALPHA);
    let p = &mut *Box::into_raw(panel);
    let title = control(hwnd, p, "STATIC", "Session", 0, 0, [24, 18, 350, 36]);
    SendMessageW(title, WM_SETFONT, p.heading as usize, 0);
    control(
        hwnd,
        p,
        "STATIC",
        "Video requests / local controls",
        0,
        0,
        [24, 60, 370, 24],
    );
    for (i, label) in [
        "Resolution",
        "Bandwidth (Mbps)",
        "Frame rate (FPS)",
        "Frame pacing",
        "Orientation",
        "Immersive mode",
    ]
    .iter()
    .enumerate()
    {
        control(
            hwnd,
            p,
            "STATIC",
            label,
            0,
            0,
            [24, 104 + i as i32 * 44, 150, 24],
        );
    }
    let res = match video.resolution {
        Resolution::Host => "Keep host".into(),
        Resolution::Client => "Match client display".into(),
        Resolution::Fixed(w, h) => format!("{w} x {h}"),
    };
    p.controls[0] = combo(
        hwnd,
        p,
        20,
        100,
        &[
            "Keep host",
            "Match client display",
            "1280 x 720",
            "1600 x 900",
            "1920 x 1080",
            "1920 x 1200",
            "2560 x 1440",
            "2560 x 1600",
            "3840 x 2160",
        ],
        &res,
        true,
    );
    p.controls[1] = combo(
        hwnd,
        p,
        21,
        144,
        &["Keep host", "5", "10", "15", "20", "30", "50", "100"],
        &video
            .bandwidth_mbps
            .map_or("Keep host".into(), |n| n.to_string()),
        true,
    );
    p.controls[2] = combo(
        hwnd,
        p,
        22,
        188,
        &["Keep host", "30", "60", "90", "120", "144", "165", "240"],
        &video.fps.map_or("Keep host".into(), |n| n.to_string()),
        true,
    );
    p.controls[3] = combo(
        hwnd,
        p,
        23,
        232,
        &["Keep host", "Adaptive", "Constant FPS"],
        match video.constant_fps {
            None => "Keep host",
            Some(false) => "Adaptive",
            Some(true) => "Constant FPS",
        },
        false,
    );
    p.controls[4] = combo(
        hwnd,
        p,
        24,
        276,
        &["Keep host", "Default", "Rotated"],
        match video.rotated {
            None => "Keep host",
            Some(false) => "Default",
            Some(true) => "Rotated",
        },
        false,
    );
    p.controls[5] = combo(
        hwnd,
        p,
        25,
        320,
        &["Disabled", "Mouse", "Keyboard", "Mouse + keyboard"],
        match video.immersive {
            1 => "Mouse + keyboard",
            2 => "Keyboard",
            3 => "Mouse",
            _ => "Disabled",
        },
        false,
    );
    p.fullscreen = control(
        hwnd,
        p,
        "BUTTON",
        "Fullscreen",
        30,
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        [24, 366, 350, 30],
    );
    SendMessageW(
        p.fullscreen,
        BM_SETCHECK,
        usize::from(window.native_fullscreen()),
        0,
    );
    control(
        hwnd,
        p,
        "STATIC",
        "Video changes are host requests. Constant FPS does not duplicate frames locally.",
        STATUS as usize,
        0,
        [24, 404, 370, 46],
    );
    for (id, label, x, y, w) in [
        (SETTINGS, "Settings", 24, 466, 178),
        (APPLY, "Apply", 214, 466, 178),
        (DISCONNECT, "Disconnect", 24, 516, 178),
        (CLOSE, "Close", 214, 516, 178),
    ] {
        control(
            hwnd,
            p,
            "BUTTON",
            label,
            id,
            BS_OWNERDRAW as u32 | WS_TABSTOP,
            [x, y, w, 36],
        );
    }
    ShowWindow(hwnd, SW_SHOW);
    raise(window);
    SetFocus(p.controls[0]);
    #[cfg(test)]
    SetPropW(
        hwnd,
        wide("ParsecWebTurn.SessionPanel.Ready").as_ptr(),
        1usize as HANDLE,
    );
}
pub fn result(window: &crate::window::Window, ok: bool) {
    unsafe {
        let panel = panel(window);
        if !panel.is_null() {
            PostMessageW(panel, RESULT, usize::from(!ok), 0);
        }
    }
}
pub fn reported(window: &crate::window::Window) {
    unsafe {
        let hwnd = panel(window);
        if !hwnd.is_null() {
            PostMessageW(hwnd, RESULT, 2, 0);
        }
    }
}
pub unsafe fn dialog(parent: HWND, msg: &MSG) -> bool {
    let h = panel_for_parent(parent);
    !h.is_null() && IsDialogMessageW(h, msg) != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editable_values_are_bounded_before_sending() {
        assert_eq!(
            resolution("3440 x 1440").unwrap(),
            Resolution::Fixed(3440, 1440)
        );
        assert_eq!(number("Keep host", 100).unwrap(), None);
        assert!(number("101", 100).is_err());
        assert!(resolution("wrong").is_err());
    }
    #[test]
    fn panel_controls_emit_native_actions_without_a_guest_or_network() {
        let window = crate::window::Window::create_native().unwrap();
        struct Close(Arc<crate::window::Window>);
        impl Drop for Close {
            fn drop(&mut self) {
                self.0.request_stop();
                self.0.close();
            }
        }
        let _close = Close(window.clone());
        let path = std::env::temp_dir().join(format!(
            "parsec-overlay-fixture-{}.json",
            crate::bud_attempt::native_attempt_id().unwrap()
        ));
        window.install_connection_settings(crate::connection_settings::Manager::open(path));
        let button_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let button = loop {
            let button = window.native_session_button.load(Ordering::Acquire) as HWND;
            if !button.is_null() {
                break button;
            }
            assert!(
                std::time::Instant::now() < button_deadline,
                "Session button did not open"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        unsafe {
            SendMessageW(button, BM_CLICK, 0, 0);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let panel = loop {
            let panel = unsafe { panel(&window) };
            if !panel.is_null()
                && !unsafe { GetPropW(panel, wide("ParsecWebTurn.SessionPanel.Ready").as_ptr()) }
                    .is_null()
            {
                break panel;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Session panel did not open"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        unsafe {
            assert_eq!(GetWindow(panel, GW_OWNER), window.handle());
            assert_ne!(GetWindowLongPtrW(panel, GWL_STYLE) as u32 & WS_POPUP, 0);
            assert_ne!(
                GetWindowLongPtrW(panel, GWL_EXSTYLE) as u32 & WS_EX_LAYERED,
                0
            );
            assert_eq!(
                GetWindowLongPtrW(panel, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                0
            );
            let mut alpha = 0u8;
            let mut key = 0u32;
            let mut flags = 0u32;
            assert_ne!(
                GetLayeredWindowAttributes(panel, &mut key, &mut alpha, &mut flags),
                0
            );
            assert_eq!(alpha, 238);
            if let Some(path) = std::env::var_os("PWT_NATIVE_UI_CAPTURE") {
                let path = std::path::PathBuf::from(path);
                super::super::ui::capture_fixture(panel, &path).unwrap();
                assert!(!GetDlgItem(button, OPEN as i32).is_null());
                let badge_path = path.with_file_name("session-button.png");
                super::super::ui::capture_fixture(button, &badge_path).unwrap();
                let badge = image::open(badge_path).unwrap().to_rgb8();
                // Check the interior, excluding the accent border: a blank
                // light rectangle cannot satisfy both text and dark fill.
                let interior: Vec<_> = badge
                    .enumerate_pixels()
                    .filter(|(x, y, _)| *x > 5 && *x < 102 && *y > 4 && *y < 29)
                    .map(|(_, _, p)| p.0)
                    .collect();
                assert!(
                    interior
                        .iter()
                        .filter(|p| p.iter().all(|v| *v > 180))
                        .count()
                        > 15,
                    "Session label was not painted"
                );
                assert!(
                    interior
                        .iter()
                        .filter(|p| p.iter().all(|v| *v < 100))
                        .count()
                        > 1000,
                    "Session background was not painted"
                );
            }
            SendMessageW(panel, WM_COMMAND, APPLY, 0);
        }
        let mut events = std::collections::VecDeque::new();
        events.clear();
        window.events.take_batch(&mut events);
        assert!(events.iter().any(|e|matches!(e,crate::window::Event::NativeSession(Action::Apply(v)) if v.resolution==Resolution::Host && v.immersive==0)));
        for (selected, mode) in [(0, 0), (1, 3), (2, 2), (3, 1)] {
            unsafe {
                for (id, index) in [(20, 2), (21, 3), (22, 2), (23, 2), (24, 2), (25, selected)] {
                    SendMessageW(GetDlgItem(panel, id), CB_SETCURSEL, index, 0);
                }
                SendMessageW(GetDlgItem(panel, 30), BM_SETCHECK, BST_CHECKED as usize, 0);
                SendMessageW(panel, RESULT, 0, 0);
                SendMessageW(panel, WM_COMMAND, APPLY, 0);
            }
            events.clear();
            window.events.take_batch(&mut events);
            assert!(events.iter().any(|event|matches!(event,crate::window::Event::NativeSession(Action::Apply(v)) if v.resolution==Resolution::Fixed(1280,720) && v.bandwidth_mbps==Some(15) && v.fps==Some(60) && v.constant_fps==Some(true) && v.rotated==Some(true) && v.immersive==mode && v.fullscreen)));
        }
        unsafe {
            SendMessageW(panel, WM_COMMAND, DISCONNECT, 0);
        }
        events.clear();
        window.events.take_batch(&mut events);
        assert!(events.iter().any(|event| matches!(
            event,
            crate::window::Event::NativeSession(Action::Disconnect)
        )));
        unsafe {
            SendMessageW(panel, WM_COMMAND, CLOSE, 0);
        }
        assert_eq!(window.native_panel.load(Ordering::Acquire), 0);
    }
}
