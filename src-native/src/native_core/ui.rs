//! Native account and computer picker; all control access stays on the UI thread.
use super::account::{self, Host, Login, Session};
use anyhow::{ensure, Result};
use std::{
    ptr::{null, null_mut},
    sync::{mpsc, Arc},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::*,
    UI::{Controls::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
const BG: u32 = 0x00231d19;
const FIELD: u32 = 0x003a322c;
const FG: u32 = 0x00f5f3f0;
const LOGIN: usize = 1;
const REFRESH: usize = 2;
const CONNECT: usize = 3;
const LOGOUT: usize = 4;
const LIST: usize = 5;
const REMEMBER: usize = 6;
const DONE: usize = 0x6e01;
pub(super) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

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
    manager: Arc<crate::connection_settings::Manager>,
    fields: [HWND; 3],
    remember: HWND,
    list: HWND,
    peer: HWND,
    status: HWND,
    login_controls: Vec<HWND>,
    picker_controls: Vec<HWND>,
    font: HFONT,
    heading: HFONT,
    brush: HBRUSH,
    field_brush: HBRUSH,
    session: Option<Arc<Session>>,
    hosts: Vec<Host>,
    incoming: Option<mpsc::Receiver<Completed>>,
    remember_login: bool,
    email: String,
    done: bool,
    result: Option<Selection>,
}
unsafe fn text(hwnd: HWND) -> String {
    let n = GetWindowTextLengthW(hwnd).clamp(0, 4096) as usize;
    let mut b = zeroize::Zeroizing::new(vec![0u16; n + 1]);
    let used = GetWindowTextW(hwnd, b.as_mut_ptr(), b.len() as i32).max(0) as usize;
    String::from_utf16_lossy(&b[..used])
}
pub(super) unsafe fn font(size: i32, weight: i32) -> HFONT {
    CreateFontW(
        -size,
        0,
        0,
        0,
        weight,
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
unsafe fn add(
    parent: HWND,
    f: &Form,
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
        WS_CHILD
            | WS_VISIBLE
            | style
            | if class == "BUTTON" && style & 0xf != BS_AUTOCHECKBOX as u32 {
                BS_OWNERDRAW as u32
            } else {
                0
            },
        r[0],
        r[1],
        r[2],
        r[3],
        parent,
        id as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    SendMessageW(h, WM_SETFONT, f.font as usize, 0);
    h
}
unsafe fn status(f: &Form, s: &str) {
    SetWindowTextW(f.status, wide(s).as_ptr());
}
unsafe fn view(f: &Form) {
    let login = f.session.is_none();
    for &h in &f.login_controls {
        ShowWindow(h, if login { SW_SHOW } else { SW_HIDE });
    }
    for &h in &f.picker_controls {
        ShowWindow(h, if login { SW_HIDE } else { SW_SHOW });
    }
    for h in f.fields.into_iter().chain([f.remember]) {
        EnableWindow(h, i32::from(f.incoming.is_none()));
    }
    for id in [LOGIN, REFRESH, CONNECT, LOGOUT] {
        let h = GetDlgItem(GetParent(f.status), id as i32);
        EnableWindow(h, i32::from(f.incoming.is_none()));
    }
}
unsafe fn start_hosts(hwnd: HWND, f: &mut Form) {
    let Some(session) = f.session.clone() else {
        return;
    };
    let (tx, rx) = mpsc::channel();
    f.incoming = Some(rx);
    status(f, "Refreshing computers...");
    view(f);
    std::thread::spawn(move || {
        let _ = tx.send(Completed::Hosts(account::hosts(&session)));
    });
    SetTimer(hwnd, DONE, 80, None);
}
unsafe fn completed(hwnd: HWND, f: &mut Form, job: Completed) {
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
            start_hosts(hwnd, f);
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
            SendMessageW(f.list, LB_RESETCONTENT, 0, 0);
            f.hosts = hosts;
            let last = f.manager.view().0.native_core.last_peer;
            let mut selected = 0;
            for (i, h) in f.hosts.iter().enumerate() {
                let name = format!("{}   /   {}", h.name, h.peer);
                SendMessageW(f.list, LB_ADDSTRING, 0, wide(&name).as_ptr() as isize);
                if h.peer == last {
                    selected = i;
                }
            }
            if !f.hosts.is_empty() {
                SendMessageW(f.list, LB_SETCURSEL, selected, 0);
                SetWindowTextW(f.peer, wide(&f.hosts[selected].peer).as_ptr());
            }
            status(
                f,
                if f.hosts.is_empty() {
                    "No computers listed. Refresh, or enter the host peer ID."
                } else {
                    "Choose a computer, or enter its peer ID. Owner connections are supported."
                },
            );
            view(f);
            SetFocus(f.list);
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
            status(f, &e.to_string());
            view(f);
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
            WM_DRAWITEM => {
                if draw_button(lp) {
                    return 1;
                }
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
                SetTextColor(wp as HDC, FG);
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
                if f.incoming.is_some() {
                    return 0;
                }
                match id {
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
                    REFRESH => start_hosts(hwnd, f),
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
        wide("ParsecWebTurn - native core").as_ptr(),
        MB_OK | MB_ICONINFORMATION,
    );
}

pub fn choose(
    manager: Arc<crate::connection_settings::Manager>,
    current: Option<Arc<Session>>,
) -> Result<Option<Selection>> {
    unsafe {
        let profile = manager.view().0.native_core;
        let (saved, notice) = match profile.session() {
            Ok(s) => (s.map(Arc::new), None),
            Err(_) => (
                None,
                Some("The saved sign-in cannot be decrypted for this Windows user. Sign in again."),
            ),
        };
        let mut f = Box::new(Form {
            manager,
            fields: [null_mut(); 3],
            remember: null_mut(),
            list: null_mut(),
            peer: null_mut(),
            status: null_mut(),
            login_controls: vec![],
            picker_controls: vec![],
            font: font(17, 400),
            heading: font(32, 600),
            brush: CreateSolidBrush(BG),
            field_brush: CreateSolidBrush(FIELD),
            session: current.or(saved),
            hosts: vec![],
            incoming: None,
            remember_login: !profile.encrypted_session.is_empty(),
            email: profile.email.clone(),
            done: false,
            result: None,
        });
        let name = wide("ParsecWebTurn.NativeCore.Launcher");
        let instance = GetModuleHandleW(null());
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: name.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..std::mem::zeroed()
        });
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            name.as_ptr(),
            wide("ParsecWebTurn - native core").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
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
        let heading = add(hwnd, &f, "STATIC", "ParsecWebTurn", 0, 0, [30, 24, 650, 46]);
        SendMessageW(heading, WM_SETFONT, f.heading as usize, 0);
        add(
            hwnd,
            &f,
            "STATIC",
            "NATIVE CORE  /  Your computers, your connection",
            0,
            0,
            [32, 80, 650, 26],
        );
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
        view(&f);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        if f.session.is_some() {
            start_hosts(hwnd, &mut f);
        } else {
            SetFocus(f.fields[0]);
        }
        let mut msg: MSG = std::mem::zeroed();
        while !f.done && GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
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
        DeleteObject(f.font);
        DeleteObject(f.heading);
        DeleteObject(f.brush);
        DeleteObject(f.field_brush);
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
    let mut label = wide(&text(item.hwndItem));
    let mut r = item.rcItem;
    DrawTextW(
        item.hDC,
        label.as_mut_ptr(),
        -1,
        &mut r,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    if item.itemState & ODS_FOCUS != 0 {
        r.left += 3;
        r.top += 3;
        r.right -= 3;
        r.bottom -= 3;
        DrawFocusRect(item.hDC, &r);
    }
    true
}
