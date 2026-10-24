//! Native account and computer picker; all control access stays on the UI thread.
use super::account::{self, Host, Login, Session};
use super::controls::{self, BG, FG, FIELD};
pub(super) use super::controls::{font, wide};
use anyhow::{ensure, Result};
use std::{
    ptr::{null, null_mut},
    sync::{mpsc, Arc},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{
        LibraryLoader::*,
        SystemServices::{SS_CENTERIMAGE, SS_ICON, SS_REALSIZEIMAGE},
    },
    UI::{Controls::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
const LOGIN: usize = 1;
const REFRESH: usize = 2;
const CONNECT: usize = 3;
const LOGOUT: usize = 4;
const LIST: usize = 5;
const REMEMBER: usize = 6;
const SETTINGS: usize = 7;
const DONE: usize = 0x6e01;
const AUTO_REFRESH: usize = 0x6e02;
const WORDMARK_BLUE: usize = 0x6e03;
#[cfg(test)]
thread_local! {static UI_FIXTURE:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}

pub struct Selection {
    pub session: Arc<Session>,
    pub peer: String,
}
enum Completed {
    Login(Result<Arc<Session>>),
    Hosts(Result<Vec<Host>>),
    Logout,
}
struct Form {
    settings_page: HWND,
    manager: Arc<crate::connection_settings::Manager>,
    fields: [HWND; 3],
    remember: HWND,
    list: HWND,
    peer: HWND,
    status: HWND,
    common_controls: Vec<HWND>,
    login_controls: Vec<HWND>,
    picker_controls: Vec<HWND>,
    font: HFONT,
    heading: HFONT,
    brush: HBRUSH,
    field_brush: HBRUSH,
    session: Option<Arc<Session>>,
    hosts: Vec<Host>,
    incoming: Option<mpsc::Receiver<Completed>>,
    background_refresh: bool,
    remember_login: bool,
    email: String,
    done: bool,
    result: Option<Selection>,
}
impl Drop for Form {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.font);
            DeleteObject(self.heading);
            DeleteObject(self.brush);
            DeleteObject(self.field_brush);
        }
    }
}
unsafe fn text(hwnd: HWND) -> String {
    controls::text(hwnd, 4096)
}
unsafe fn add(
    parent: HWND,
    f: &Form,
    class: &str,
    label: &str,
    id: usize,
    style: u32,
    r: [i32; 4],
) -> HWND {
    let owner_draw = class == "BUTTON" && style & 0xf != BS_AUTOCHECKBOX as u32;
    let style = style | if owner_draw { BS_OWNERDRAW as u32 } else { 0 };
    controls::create(parent, f.font, class, label, id, style, r)
}
unsafe fn status(f: &Form, s: &str) {
    SetWindowTextW(f.status, wide(s).as_ptr());
}
unsafe fn view(f: &Form) {
    let launcher = f.settings_page.is_null();
    let login = f.session.is_none();
    for &h in &f.common_controls {
        ShowWindow(h, if launcher { SW_SHOW } else { SW_HIDE });
    }
    for &h in &f.login_controls {
        ShowWindow(h, if launcher && login { SW_SHOW } else { SW_HIDE });
    }
    for &h in &f.picker_controls {
        ShowWindow(h, if launcher && !login { SW_SHOW } else { SW_HIDE });
    }
    let busy = f.incoming.is_some() && !f.background_refresh;
    if f.session.is_some() {
        SetTimer(GetParent(f.status), AUTO_REFRESH, 30_000, None);
    } else {
        KillTimer(GetParent(f.status), AUTO_REFRESH);
    }
    for h in f.fields.into_iter().chain([f.remember]) {
        EnableWindow(h, i32::from(!busy));
    }
    for id in [LOGIN, REFRESH, CONNECT, LOGOUT, SETTINGS] {
        let h = GetDlgItem(GetParent(f.status), id as i32);
        EnableWindow(h, i32::from(!busy));
    }
}
unsafe fn start_hosts(hwnd: HWND, f: &mut Form, quiet: bool) {
    if f.incoming.is_some() {
        return;
    }
    let Some(session) = f.session.clone() else {
        return;
    };
    let (tx, rx) = mpsc::channel();
    f.incoming = Some(rx);
    f.background_refresh = quiet;
    if !quiet {
        status(f, "Refreshing computers...");
        view(f);
    }
    std::thread::spawn(move || {
        let _ = tx.send(Completed::Hosts(account::hosts(&session)));
    });
    SetTimer(hwnd, DONE, if quiet { 500 } else { 80 }, None);
}
unsafe fn completed(hwnd: HWND, f: &mut Form, job: Completed) {
    let quiet = std::mem::replace(&mut f.background_refresh, false);
    match job {
        Completed::Login(Ok(session)) => {
            if !f.remember_login {
                if f.manager
                    .update_native(|p| p.encrypted_session.clear())
                    .is_err()
                {
                    message_box(hwnd,"The old saved sign-in could not be removed. Check write access to settings.json.");
                }
            }
            if f.remember_login {
                let save = super::profile::Profile::encrypted(session.token()).and_then(|blob| {
                    f.manager.update_native(|p| {
                        p.email = f.email.clone();
                        p.encrypted_session = blob;
                    })
                });
                if save.is_ok() {
                    session.remembered();
                }
                if save.is_err() {
                    message_box(hwnd,"Signed in, but Windows could not save the protected session. This sign-in will stay in memory.");
                    f.remember_login = false;
                }
            }
            f.session = Some(session);
            start_hosts(hwnd, f, false);
        }
        Completed::Login(Err(e)) => {
            crate::event_log::record(
                "ERROR",
                "native_account_login_failed",
                serde_json::json!({}),
            );
            status(f, &e.to_string());
            view(f);
            SetFocus(f.fields[1]);
        }
        Completed::Hosts(Ok(hosts)) => {
            let changed = !f
                .hosts
                .iter()
                .map(|h| (&h.peer, &h.name))
                .eq(hosts.iter().map(|h| (&h.peer, &h.name)));
            if changed {
                let old_index = SendMessageW(f.list, LB_GETCURSEL, 0, 0);
                let selected_peer = usize::try_from(old_index)
                    .ok()
                    .and_then(|i| f.hosts.get(i))
                    .map(|h| h.peer.clone())
                    .unwrap_or_else(|| f.manager.view().0.native_core.last_peer);
                let top = SendMessageW(f.list, LB_GETTOPINDEX, 0, 0).max(0) as usize;
                let redraw = GetWindowLongW(f.list, GWL_STYLE) as u32 & WS_VISIBLE != 0;
                if redraw {
                    SendMessageW(f.list, WM_SETREDRAW, 0, 0);
                }
                SendMessageW(f.list, LB_RESETCONTENT, 0, 0);
                f.hosts = hosts;
                let mut selected = 0;
                for (i, h) in f.hosts.iter().enumerate() {
                    let name = format!("{}   /   {}", h.name, h.peer);
                    SendMessageW(f.list, LB_ADDSTRING, 0, wide(&name).as_ptr() as isize);
                    if h.peer == selected_peer {
                        selected = i;
                    }
                }
                if !f.hosts.is_empty() {
                    SendMessageW(f.list, LB_SETCURSEL, selected, 0);
                    SendMessageW(f.list, LB_SETTOPINDEX, top.min(f.hosts.len() - 1), 0);
                    if !quiet {
                        SetWindowTextW(f.peer, wide(&f.hosts[selected].peer).as_ptr());
                    }
                }
                if redraw {
                    SendMessageW(f.list, WM_SETREDRAW, 1, 0);
                    RedrawWindow(f.list, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE);
                }
            }
            if !quiet {
                status(
                    f,
                    if f.hosts.is_empty() {
                        "No computers listed. Refresh, or enter the host peer ID."
                    } else {
                        "Choose a computer, or enter its peer ID. Owner connections are supported."
                    },
                );
                view(f);
                if f.settings_page.is_null() {
                    SetFocus(f.list);
                }
            }
        }
        Completed::Hosts(Err(e)) => {
            crate::event_log::record(
                "WARN",
                "native_computer_list_failed",
                serde_json::json!({"session_rejected":e.downcast_ref::<account::ExpiredSession>().is_some()}),
            );
            if e.downcast_ref::<account::ExpiredSession>().is_some() {
                let _ = f.manager.update_native(|p| p.encrypted_session.clear());
                f.session = None;
                f.remember_login = false;
            }
            if !quiet || f.session.is_none() {
                status(f, &e.to_string());
                view(f);
            }
        }
        Completed::Logout => {
            status(f, "Signed out. Enter your account details to sign in.");
            view(f);
            SetFocus(f.fields[0]);
        }
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
        );
    }
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Form;
    if !p.is_null() {
        let f = &mut *p;
        match msg {
            m if m == WM_APP + 9 => {
                let page = std::mem::replace(&mut f.settings_page, null_mut());
                if !page.is_null() {
                    DestroyWindow(page);
                }
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    0,
                    0,
                    760,
                    610,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                view(f);
                InvalidateRect(hwnd, null(), 1);
                SetFocus(if f.session.is_some() {
                    f.list
                } else {
                    f.fields[0]
                });
                return 0;
            }

            WM_DRAWITEM => {
                if draw_button(lp) {
                    return 1;
                }
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
                SetTextColor(
                    wp as HDC,
                    if GetDlgCtrlID(lp as HWND) == WORDMARK_BLUE as i32 {
                        crate::branding::BLUE
                    } else {
                        FG
                    },
                );
                SetBkColor(wp as HDC, if msg == WM_CTLCOLORSTATIC { BG } else { FIELD });
                return if msg == WM_CTLCOLORSTATIC {
                    f.brush
                } else {
                    f.field_brush
                } as isize;
            }
            WM_ERASEBKGND => {
                let mut r = std::mem::zeroed();
                GetClientRect(hwnd, &mut r);
                FillRect(wp as HDC, &r, f.brush);
                return 1;
            }
            WM_TIMER if wp == AUTO_REFRESH => {
                if f.session.is_some() && f.incoming.is_none() && !f.done {
                    start_hosts(hwnd, f, true);
                }
                return 0;
            }
            WM_TIMER if wp == DONE => {
                let result = f.incoming.as_ref().map(|r| r.try_recv());
                match result {
                    Some(Ok(job)) => {
                        f.incoming = None;
                        KillTimer(hwnd, DONE);
                        completed(hwnd, f, job);
                    }
                    Some(Err(mpsc::TryRecvError::Disconnected)) => {
                        f.incoming = None;
                        KillTimer(hwnd, DONE);
                        status(f, "The account request ended unexpectedly. Please retry.");
                        view(f);
                    }
                    _ => {}
                }
                return 0;
            }
            WM_COMMAND => {
                let id = wp & 0xffff;
                if id == LIST && (wp >> 16) as u32 == LBN_SELCHANGE {
                    let n = SendMessageW(f.list, LB_GETCURSEL, 0, 0);
                    if let Some(h) = usize::try_from(n).ok().and_then(|i| f.hosts.get(i)) {
                        SetWindowTextW(f.peer, wide(&h.peer).as_ptr());
                    }
                }
                if f.incoming.is_some() && !f.background_refresh {
                    return 0;
                }
                match id {
                    SETTINGS => {
                        if f.settings_page.is_null() {
                            f.settings_page =
                                crate::connection_settings_ui::open(hwnd, f.manager.clone());
                        }
                        if !f.settings_page.is_null() {
                            view(f);
                            SetWindowPos(
                                f.settings_page,
                                HWND_TOP,
                                0,
                                0,
                                0,
                                0,
                                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                            );
                            SetFocus(f.settings_page);
                        }
                    }

                    LOGIN => {
                        let email = text(f.fields[0]);
                        let password = zeroize::Zeroizing::new(text(f.fields[1]));
                        let tfa = zeroize::Zeroizing::new(text(f.fields[2]));
                        if email.trim().is_empty() || password.is_empty() {
                            status(f, "Email and password are required.");
                            return 0;
                        }
                        f.email = email.trim().into();
                        f.remember_login =
                            SendMessageW(f.remember, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
                        let login = Login {
                            email,
                            password,
                            tfa,
                        };
                        SetWindowTextW(f.fields[1], wide("").as_ptr());
                        SetWindowTextW(f.fields[2], wide("").as_ptr());
                        let (tx, rx) = mpsc::channel();
                        f.incoming = Some(rx);
                        status(f, "Signing in securely...");
                        view(f);
                        std::thread::spawn(move || {
                            let result = account::login(&login).map(Arc::new);
                            if let Err(mpsc::SendError(Completed::Login(Ok(session)))) =
                                tx.send(Completed::Login(result))
                            {
                                session.revoke();
                            }
                        });
                        SetTimer(hwnd, DONE, 80, None);
                    }
                    REFRESH => start_hosts(hwnd, f, false),
                    CONNECT => {
                        let peer = text(f.peer).trim().to_owned();
                        if peer.is_empty() || peer.len() > 256 || peer.chars().any(char::is_control)
                        {
                            status(f, "Select a computer or enter a valid host peer ID.");
                            return 0;
                        }
                        if let Some(session) = f.session.take() {
                            if f.manager
                                .update_native(|p| p.last_peer = peer.clone())
                                .is_err()
                            {
                                message_box(hwnd,"Could not save the selected computer. Your connection can still continue.");
                            }
                            f.result = Some(Selection { session, peer });
                            DestroyWindow(hwnd);
                        }
                    }
                    LOGOUT => {
                        if f.manager
                            .update_native(|p| {
                                p.encrypted_session.clear();
                                p.email.clear();
                            })
                            .is_err()
                        {
                            status(f,"Could not remove the saved sign-in. Check write access to settings.json.");
                            return 0;
                        }
                        f.remember_login = false;
                        if let Some(session) = f.session.take() {
                            let (tx, rx) = mpsc::channel();
                            f.incoming = Some(rx);
                            f.background_refresh = false;
                            status(f, "Signing out...");
                            view(f);
                            std::thread::spawn(move || {
                                session.revoke();
                                let _ = tx.send(Completed::Logout);
                            });
                            SetTimer(hwnd, DONE, 80, None);
                        }
                    }
                    _ => {}
                }
                return 0;
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                return 0;
            }
            WM_DESTROY => {
                f.done = true;
                KillTimer(hwnd, DONE);
                KillTimer(hwnd, AUTO_REFRESH);
                return 0;
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
pub unsafe fn message_box(owner: HWND, message: &str) {
    MessageBoxW(
        owner,
        wide(message).as_ptr(),
        wide("EvoParseq").as_ptr(),
        MB_OK | MB_ICONINFORMATION,
    );
}

pub fn choose(
    manager: Arc<crate::connection_settings::Manager>,
    current: Option<Arc<Session>>,
) -> Result<Option<Selection>> {
    unsafe {
        let profile = manager.view().0.native_core;
        let (saved, notice) = if current.is_some() {
            (None, None)
        } else {
            match profile.session() {
            Ok(s) => (s.map(Arc::new), None),
            Err(_) => (
                None,
                Some("The saved sign-in cannot be decrypted for this Windows user. Sign in again."),
            ),
        }
        };
        let mut f = Box::new(Form {
            settings_page: null_mut(),
            manager,
            fields: [null_mut(); 3],
            remember: null_mut(),
            list: null_mut(),
            peer: null_mut(),
            status: null_mut(),
            common_controls: vec![],
            login_controls: vec![],
            picker_controls: vec![],
            font: font(17, 400),
            heading: font(32, 600),
            brush: CreateSolidBrush(BG),
            field_brush: CreateSolidBrush(FIELD),
            session: current.or(saved),
            hosts: vec![],
            incoming: None,
            background_refresh: false,
            remember_login: !profile.encrypted_session.is_empty(),
            email: profile.email.clone(),
            done: false,
            result: None,
        });
        let name = wide("EvoParseq.NativeCore.Launcher");
        let instance = GetModuleHandleW(null());
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            hIcon: crate::branding::icon(),
            lpszClassName: name.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..std::mem::zeroed()
        });
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            name.as_ptr(),
            wide("EvoParseq").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            760,
            610,
            null_mut(),
            null_mut(),
            instance,
            (&mut *f as *mut Form).cast(),
        );
        ensure!(!hwnd.is_null(), "Native account window unavailable");
        let mark = add(
            hwnd,
            &f,
            "STATIC",
            "",
            8,
            SS_ICON | SS_CENTERIMAGE | SS_REALSIZEIMAGE,
            [32, 24, 40, 46],
        );
        SendMessageW(
            mark,
            STM_SETICON,
            crate::branding::launcher_icon() as usize,
            0,
        );
        f.common_controls.push(mark);
        for (label, id, rect) in [
            ("EVO", 0, [84, 24, 68, 46]),
            ("PARSEQ", WORDMARK_BLUE, [152, 24, 180, 46]),
        ] {
            let heading = add(hwnd, &f, "STATIC", label, id, SS_CENTERIMAGE, rect);
            SendMessageW(heading, WM_SETFONT, f.heading as usize, 0);
            f.common_controls.push(heading);
        }
        let settings = add(
            hwnd,
            &f,
            "BUTTON",
            "Settings",
            SETTINGS,
            WS_TABSTOP,
            [580, 28, 116, 36],
        );
        f.common_controls.push(settings);
        let subtitle = add(
            hwnd,
            &f,
            "STATIC",
            "NATIVE CORE  /  Your computers, your connection",
            0,
            0,
            [32, 80, 650, 26],
        );
        f.common_controls.push(subtitle);
        for (i, label) in ["Email", "Password", "Two-factor code (optional)"]
            .iter()
            .enumerate()
        {
            let y = 136 + i as i32 * 60;
            let l = add(hwnd, &f, "STATIC", label, 0, 0, [32, y, 215, 26]);
            let edit = add(
                hwnd,
                &f,
                "EDIT",
                "",
                10 + i,
                WS_TABSTOP
                    | WS_BORDER
                    | ES_AUTOHSCROLL as u32
                    | if i == 1 { ES_PASSWORD as u32 } else { 0 },
                [250, y - 2, 446, 32],
            );
            SendMessageW(edit, EM_SETLIMITTEXT, 4096, 0);
            f.fields[i] = edit;
            f.login_controls.extend([l, edit]);
        }
        SetWindowTextW(f.fields[0], wide(&profile.email).as_ptr());
        f.remember = add(
            hwnd,
            &f,
            "BUTTON",
            "Remember sign-in for this Windows user",
            REMEMBER,
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            [250, 322, 450, 30],
        );
        SendMessageW(f.remember, BM_SETCHECK, usize::from(f.remember_login), 0);
        f.login_controls.push(f.remember);
        let b = add(
            hwnd,
            &f,
            "BUTTON",
            "Sign in",
            LOGIN,
            WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            [490, 370, 206, 40],
        );
        f.login_controls.push(b);
        let list = add(
            hwnd,
            &f,
            "LISTBOX",
            "",
            LIST,
            WS_TABSTOP | WS_BORDER | WS_VSCROLL | LBS_NOTIFY as u32,
            [32, 130, 664, 220],
        );
        f.list = list;
        f.picker_controls.push(list);
        let l = add(hwnd, &f, "STATIC", "Host peer ID", 0, 0, [32, 370, 155, 26]);
        f.peer = add(
            hwnd,
            &f,
            "EDIT",
            &profile.last_peer,
            20,
            WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
            [200, 366, 496, 32],
        );
        SendMessageW(f.peer, EM_SETLIMITTEXT, 256, 0);
        f.picker_controls.extend([l, f.peer]);
        for (id, label, x, w) in [
            (REFRESH, "Refresh", 32, 140),
            (LOGOUT, "Sign out", 184, 140),
            (CONNECT, "Connect", 490, 206),
        ] {
            let b = add(hwnd, &f, "BUTTON", label, id, WS_TABSTOP, [x, 420, w, 40]);
            f.picker_controls.push(b);
        }
        f.status = add(
            hwnd,
            &f,
            "STATIC",
            notice.unwrap_or(
                "Use your Parsec account. Passwords and two-factor codes are never saved.",
            ),
            0,
            0,
            [32, 484, 664, 66],
        );
        f.common_controls.push(f.status);
        view(&f);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        if f.session.is_some() {
            start_hosts(hwnd, &mut f, false);
        } else {
            SetFocus(f.fields[0]);
        }
        #[cfg(test)]
        if UI_FIXTURE.with(|f| f.get()) {
            if let Some(path) = std::env::var_os("PWT_NATIVE_UI_CAPTURE") {
                capture_fixture(
                    hwnd,
                    &std::path::PathBuf::from(&path).with_file_name("sign-in.png"),
                )
                .unwrap();
            }
            for picker in [false, true] {
                if picker {
                    // A local fixture token never enters auth, host discovery,
                    // persistence or explicit logout requests.
                    f.session = Some(Arc::new(Session::from_token("offline-ui-fixture").unwrap()));
                    completed(
                        hwnd,
                        &mut f,
                        Completed::Hosts(Ok(vec![Host {
                            peer: "fixture-host".into(),
                            name: "Fixture computer".into(),
                        }])),
                    );
                }
                let selected = text(f.peer);
                SendMessageW(hwnd, WM_COMMAND, SETTINGS, 0);
                assert!(!f.settings_page.is_null());
                assert_ne!(IsWindowVisible(f.settings_page), 0);
                if picker {
                    SetWindowTextW(f.peer, wide("manually-entered-host").as_ptr());
                    f.background_refresh = true;
                    completed(
                        hwnd,
                        &mut f,
                        Completed::Hosts(Ok(vec![Host {
                            peer: "fixture-host".into(),
                            name: "Updated computer".into(),
                        }])),
                    );
                    assert_eq!(text(f.peer), "manually-entered-host");
                    f.background_refresh = true;
                    completed(
                        hwnd,
                        &mut f,
                        Completed::Hosts(Ok(vec![Host {
                            peer: "fixture-host".into(),
                            name: "Updated computer".into(),
                        }])),
                    );
                    assert_eq!(text(f.peer), "manually-entered-host");
                    SetWindowTextW(f.peer, wide(&selected).as_ptr());
                }
                // Refreshing the launcher state must not resurrect its controls
                // while Settings owns the view.
                view(&f);
                for &control in f
                    .common_controls
                    .iter()
                    .chain(&f.login_controls)
                    .chain(&f.picker_controls)
                {
                    assert_eq!(
                        IsWindowVisible(control),
                        0,
                        "Launcher control covers Settings"
                    );
                }
                if let Some(path) = std::env::var_os("PWT_NATIVE_UI_CAPTURE") {
                    capture_fixture(
                        hwnd,
                        &std::path::PathBuf::from(&path).with_file_name(if picker {
                            "picker-settings.png"
                        } else {
                            "sign-in-settings.png"
                        }),
                    )
                    .unwrap();
                    if picker {
                        SendMessageW(f.settings_page, WM_COMMAND, 4121, 0);
                        assert!(!GetDlgItem(f.settings_page, 4135).is_null());
                        SendMessageW(f.settings_page, WM_VSCROLL, SB_BOTTOM as usize, 0);
                        capture_fixture(
                            hwnd,
                            &std::path::PathBuf::from(&path)
                                .with_file_name("native-video-settings.png"),
                        )
                        .unwrap();
                    }
                }
                SendMessageW(hwnd, WM_APP + 9, 0, 0);
                assert!(f.settings_page.is_null());
                for &control in &f.common_controls {
                    assert_ne!(IsWindowVisible(control), 0);
                }
                for &control in &f.login_controls {
                    assert_eq!(IsWindowVisible(control) != 0, !picker);
                }
                for &control in &f.picker_controls {
                    assert_eq!(IsWindowVisible(control) != 0, picker);
                }
                assert_eq!(text(f.peer), selected);
            }
            f.session = None; // Do not run explicit account logout for the fixture.
            DestroyWindow(hwnd);
        }
        let mut msg: MSG = std::mem::zeroed();
        while !f.done && GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if !f.settings_page.is_null() && IsDialogMessageW(f.settings_page, &msg) != 0 {
                continue;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
        if let Some(session) = f.session.take() {
            if f.manager.view().0.native_core.encrypted_session.is_empty() {
                session.revoke();
            }
        }
        Ok(f.result.take())
    }
}

pub(crate) unsafe fn draw_button(lp: LPARAM) -> bool {
    if lp == 0 {
        return false;
    }
    let item = &*(lp as *const DRAWITEMSTRUCT);
    if item.CtlType != ODT_BUTTON {
        return false;
    }
    let selected = item.itemState & ODS_SELECTED != 0;
    let disabled = item.itemState & ODS_DISABLED != 0;
    let bg = CreateSolidBrush(if selected { 0x00674c32 } else { FIELD });
    FillRect(item.hDC, &item.rcItem, bg);
    DeleteObject(bg);
    let edge = CreateSolidBrush(0x00e7bd63);
    FrameRect(item.hDC, &item.rcItem, edge);
    DeleteObject(edge);
    SetBkMode(item.hDC, TRANSPARENT as i32);
    SetTextColor(item.hDC, if disabled { 0x00888888 } else { FG });
    let mut r = item.rcItem;
    if item.CtlID == super::session_ui::OPEN as u32 {
        DrawIconEx(
            item.hDC,
            r.left + (r.right - r.left - 32) / 2,
            r.top + (r.bottom - r.top - 32) / 2,
            crate::branding::icon(),
            32,
            32,
            0,
            null_mut(),
            DI_NORMAL,
        );
    } else {
        let mut label = wide(&text(item.hwndItem));
        DrawTextW(
            item.hDC,
            label.as_mut_ptr(),
            -1,
            &mut r,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }
    if item.itemState & ODS_FOCUS != 0 {
        r.left += 3;
        r.top += 3;
        r.right -= 3;
        r.bottom -= 3;
        DrawFocusRect(item.hDC, &r);
    }
    true
}

/// Offline GDI fixture capture only. Never called by a live video path.
#[cfg(test)]
pub(super) unsafe fn capture_fixture(hwnd: HWND, path: &std::path::Path) -> Result<()> {
    let mut r = std::mem::zeroed();
    GetClientRect(hwnd, &mut r);
    let (w, h) = (r.right as u32, r.bottom as u32);
    ensure!(
        w > 0 && h > 0 && w <= 2048 && h <= 2048,
        "Invalid UI fixture size"
    );
    let dc = CreateCompatibleDC(null_mut());
    ensure!(!dc.is_null(), "UI fixture DC unavailable");
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        },
        ..std::mem::zeroed()
    };
    let mut bits = null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() {
        DeleteDC(dc);
        anyhow::bail!("UI fixture bitmap unavailable");
    }
    let old = SelectObject(dc, bitmap);
    SendMessageW(
        hwnd,
        WM_PRINT,
        dc as usize,
        (PRF_CLIENT | PRF_CHILDREN | PRF_ERASEBKGND) as isize,
    );
    GdiFlush();
    let mut bytes = std::slice::from_raw_parts(bits.cast::<u8>(), (w * h * 4) as usize).to_vec();
    SelectObject(dc, old);
    DeleteObject(bitmap);
    DeleteDC(dc);
    for p in bytes.chunks_exact_mut(4) {
        p.swap(0, 2);
        p[3] = 255;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image::RgbaImage::from_raw(w, h, bytes)
        .unwrap()
        .save(path)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launcher_can_render_without_credentials_or_external_requests() {
        let path = std::env::temp_dir().join(format!(
            "parsec-launcher-fixture-{}.json",
            crate::bud_attempt::native_attempt_id().unwrap()
        ));
        let manager = crate::connection_settings::Manager::open(path);
        UI_FIXTURE.with(|f| f.set(true));
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                UI_FIXTURE.with(|f| f.set(false));
            }
        }
        let _reset = Reset;
        assert!(choose(manager, None).unwrap().is_none());
    }
}
