//! Event-driven native session panel. GDI controls overlay the video child;
//! no video pixels, timer, render loop or additional decoded-frame copy.
use super::{
    profile::{Resolution, Video},
    ui::{draw_button, wide},
};
use anyhow::{ensure, Result};
use std::{
    ptr::{null, null_mut},
    sync::Arc,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::*,
    UI::{Controls::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
pub const OPEN: usize = 4600;
const PANEL: i32 = 4601;
const APPLY: usize = 4602;
const CLOSE: usize = 4603;
const DISCONNECT: usize = 4604;
const SETTINGS: usize = 4605;
const STATUS: i32 = 4606;
pub const RESULT: u32 = WM_APP + 26;
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
                    wide(if wp == 0 {
                        "Applied locally; video settings requested from host."
                    } else {
                        "Could not apply or save settings. See the event log."
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
                    CLOSE => {
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
pub unsafe fn install(window: &crate::window::Window) {
    let parent = window.handle();
    if GetDlgItem(parent, OPEN as i32).is_null() {
        CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("Session").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
            16,
            16,
            108,
            34,
            parent,
            OPEN as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
    }
    raise(window);
}
pub unsafe fn raise(window: &crate::window::Window) {
    let parent = window.handle();
    let button = GetDlgItem(parent, OPEN as i32);
    if !button.is_null() {
        ShowWindow(
            button,
            if window.settings_open() {
                SW_HIDE
            } else {
                SW_SHOWNA
            },
        );
        SetWindowPos(button, HWND_TOP, 16, 16, 108, 34, SWP_NOACTIVATE);
    }
    let panel = GetDlgItem(parent, PANEL);
    if !panel.is_null() {
        let mut r = std::mem::zeroed();
        GetClientRect(parent, &mut r);
        SetWindowPos(
            panel,
            HWND_TOP,
            16,
            60,
            420,
            (r.bottom - 76).clamp(120, 580),
            SWP_NOACTIVATE,
        );
    }
}
pub unsafe fn toggle(
    window: &crate::window::Window,
    manager: Arc<crate::connection_settings::Manager>,
) {
    let parent = window.handle();
    let existing = GetDlgItem(parent, PANEL);
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
        WS_EX_CONTROLPARENT | WS_EX_COMPOSITED,
        name.as_ptr(),
        wide("Session controls").as_ptr(),
        WS_CHILD | WS_CLIPCHILDREN | WS_VSCROLL,
        16,
        60,
        420,
        580,
        parent,
        PANEL as HMENU,
        instance,
        pointer.cast(),
    );
    if hwnd.is_null() {
        return;
    }
    panel.window_owned = true;
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
}
pub fn result(window: &crate::window::Window, ok: bool) {
    unsafe {
        let panel = GetDlgItem(window.handle(), PANEL);
        if !panel.is_null() {
            PostMessageW(panel, RESULT, usize::from(!ok), 0);
        }
    }
}
pub unsafe fn dialog(parent: HWND, msg: &MSG) -> bool {
    let h = GetDlgItem(parent, PANEL);
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
        window.native_toggle_overlay();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let panel = loop {
            let panel = unsafe { GetDlgItem(window.handle(), PANEL) };
            if !panel.is_null() && !unsafe { GetDlgItem(panel, APPLY as i32) }.is_null() {
                break panel;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Session panel did not open"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        unsafe {
            SendMessageW(panel, WM_COMMAND, APPLY, 0);
        }
        let mut events = std::collections::VecDeque::new();
        window.events.take_batch(&mut events);
        assert!(events.iter().any(|e|matches!(e,crate::window::Event::NativeSession(Action::Apply(v)) if v.resolution==Resolution::Host && v.immersive==0)));
        unsafe {
            SendMessageW(panel, WM_COMMAND, CLOSE, 0);
        }
        assert!(unsafe { GetDlgItem(window.handle(), PANEL) }.is_null());
    }
}
