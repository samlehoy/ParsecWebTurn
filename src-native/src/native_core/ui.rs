//! Small native bootstrap UI; no browser, WASM, toolkit or password on argv.
use super::account::Login;
use anyhow::{ensure, Result};
use windows_sys::Win32::Graphics::Gdi::COLOR_WINDOW;
use windows_sys::Win32::UI::Controls::EM_SETLIMITTEXT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::{Foundation::*, System::LibraryLoader::*, UI::WindowsAndMessaging::*};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
struct Form {
    fields: [HWND; 4],
    done: bool,
    result: Option<Login>,
}
unsafe fn text(hwnd: HWND) -> String {
    let length = GetWindowTextLengthW(hwnd).clamp(0, 4096) as usize;
    let mut bytes = vec![0u16; length + 1];
    let used = GetWindowTextW(hwnd, bytes.as_mut_ptr(), bytes.len() as i32).max(0) as usize;
    String::from_utf16_lossy(&bytes[..used])
}
unsafe extern "system" fn proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Form;
    if !pointer.is_null() {
        let form = &mut *pointer;
        match message {
            WM_COMMAND if wp & 0xffff == 1 => {
                let [email, password, tfa, peer] = form.fields.map(|h| text(h));
                if email.trim().is_empty() || password.is_empty() || peer.trim().is_empty() {
                    message_box(hwnd, "Enter email, password and the host peer ID.");
                    return 0;
                }
                form.result = Some(Login {
                    email,
                    password: zeroize::Zeroizing::new(password),
                    tfa: zeroize::Zeroizing::new(tfa),
                    peer: peer.trim().into(),
                });
                // Clear the password control before its native storage is freed.
                SetWindowTextW(form.fields[1], wide("").as_ptr());
                DestroyWindow(hwnd);
                return 0;
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                return 0;
            }
            WM_DESTROY => {
                form.done = true;
                return 0;
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, message, wp, lp)
}
pub unsafe fn message_box(owner: HWND, message: &str) {
    MessageBoxW(
        owner,
        wide(message).as_ptr(),
        wide("ParsecWebTurn - native core").as_ptr(),
        MB_OK | MB_ICONINFORMATION,
    );
}

pub fn login() -> Result<Option<Login>> {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let name = wide("ParsecWebTurn.NativeCore.Login");
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: name.as_ptr(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as _,
            ..std::mem::zeroed()
        };
        RegisterClassW(&class);
        let mut form = Box::new(Form {
            fields: [std::ptr::null_mut(); 4],
            done: false,
            result: None,
        });
        let hwnd = CreateWindowExW(
            0,
            name.as_ptr(),
            wide("ParsecWebTurn - native BUD core (experimental)").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            600,
            370,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            (&mut *form as *mut Form).cast(),
        );
        ensure!(!hwnd.is_null(), "Native sign-in window unavailable");
        for (index, label) in [
            "Email",
            "Password",
            "Two-factor code (optional)",
            "Host peer ID",
        ]
        .iter()
        .enumerate()
        {
            let y = 58 + index as i32 * 44;
            CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide(label).as_ptr(),
                WS_CHILD | WS_VISIBLE,
                20,
                y,
                200,
                25,
                hwnd,
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            let edit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                wide("EDIT").as_ptr(),
                wide("").as_ptr(),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_TABSTOP
                    | ES_AUTOHSCROLL as u32
                    | if index == 1 { ES_PASSWORD as u32 } else { 0 },
                225,
                y - 3,
                330,
                28,
                hwnd,
                (10 + index) as HMENU,
                instance,
                std::ptr::null(),
            );
            SendMessageW(
                edit,
                EM_SETLIMITTEXT,
                if index == 3 { 256 } else { 4096 },
                0,
            );
            form.fields[index] = edit;
        }
        CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide("Independent Rust core. BUD only. Credentials stay in memory.").as_ptr(),
            WS_CHILD | WS_VISIBLE,
            20,
            16,
            550,
            30,
            hwnd,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("Sign in and connect").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            350,
            250,
            205,
            35,
            hwnd,
            1usize as HMENU,
            instance,
            std::ptr::null(),
        );
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(form.fields[0]);
        let mut msg: MSG = std::mem::zeroed();
        while !form.done && GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
        Ok(form.result.take())
    }
}
