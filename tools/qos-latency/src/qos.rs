//! Optional qWAVE flow, once per connected socket. No rate shaping or probing.
use serde::Serialize;
use std::net::TcpStream;

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub requested: bool,
    pub applied: bool,
    pub configured_dscp: Option<u32>,
    pub configured_l2: Option<u32>,
    pub error: Option<u32>,
}
pub struct Flow {
    pub status: Status,
    #[cfg(windows)]
    handle: usize,
}
impl Flow {
    pub fn new(socket: &TcpStream, requested: bool) -> Self {
        let status = Status {
            requested,
            applied: false,
            configured_dscp: None,
            configured_l2: None,
            error: None,
        };
        #[cfg(windows)]
        {
            windows::create(socket, status)
        }
        #[cfg(not(windows))]
        {
            let _ = socket;
            Self { status }
        }
    }
}
#[cfg(windows)]
mod windows {
    use super::*;
    use std::{ffi::c_void, os::windows::io::AsRawSocket, sync::OnceLock};
    use windows_sys::Win32::{Foundation::GetLastError, System::LibraryLoader::*};
    type Handle = *mut c_void;
    #[repr(C)]
    struct Version {
        major: u16,
        minor: u16,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Priority {
        dscp: u32,
        _nonconformant_dscp: u32,
        l2: u32,
        _nonconformant_l2: u32,
    }
    type Create = unsafe extern "system" fn(*const Version, *mut Handle) -> i32;
    type Add = unsafe extern "system" fn(Handle, usize, *const c_void, i32, u32, *mut u32) -> i32;
    type Query = unsafe extern "system" fn(
        Handle,
        u32,
        i32,
        *mut u32,
        *mut Priority,
        u32,
        *mut c_void,
    ) -> i32;
    type Close = unsafe extern "system" fn(Handle) -> i32;
    struct Api {
        create: Create,
        add: Add,
        query: Query,
        close: Close,
    }
    fn api() -> Option<&'static Api> {
        static API: OnceLock<Option<Api>> = OnceLock::new();
        API.get_or_init(|| unsafe {
            let name: Vec<_> = "qwave.dll\0".encode_utf16().collect();
            // Only the system library; successful resolution lives for the process.
            let library = LoadLibraryExW(
                name.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            );
            if library.is_null() {
                return None;
            }
            let found = (|| {
                Some(Api {
                    create: std::mem::transmute::<unsafe extern "system" fn() -> isize, Create>(
                        GetProcAddress(library, c"QOSCreateHandle".as_ptr().cast())?,
                    ),
                    add: std::mem::transmute::<unsafe extern "system" fn() -> isize, Add>(
                        GetProcAddress(library, c"QOSAddSocketToFlow".as_ptr().cast())?,
                    ),
                    query: std::mem::transmute::<unsafe extern "system" fn() -> isize, Query>(
                        GetProcAddress(library, c"QOSQueryFlow".as_ptr().cast())?,
                    ),
                    close: std::mem::transmute::<unsafe extern "system" fn() -> isize, Close>(
                        GetProcAddress(library, c"QOSCloseHandle".as_ptr().cast())?,
                    ),
                })
            })();
            if found.is_none() {
                windows_sys::Win32::Foundation::FreeLibrary(library);
            }
            found
        })
        .as_ref()
    }
    pub(super) fn create(socket: &TcpStream, status: Status) -> Flow {
        let mut flow = Flow { status, handle: 0 };
        if !flow.status.requested {
            return flow;
        }
        let Some(api) = api() else {
            flow.status.error = Some(126);
            return flow;
        };
        unsafe {
            let mut handle = std::ptr::null_mut();
            if (api.create)(&Version { major: 1, minor: 0 }, &mut handle) == 0 {
                flow.status.error = Some(GetLastError());
                return flow;
            }
            flow.handle = handle as usize;
            let mut id = 0;
            // Connected TCP: never reconnect/disconnect it to apply a mark.
            // AudioVideo=3; NON_ADAPTIVE=2: no path probes or bandwidth shaping.
            if (api.add)(
                handle,
                socket.as_raw_socket() as usize,
                std::ptr::null(),
                3,
                2,
                &mut id,
            ) == 0
            {
                flow.status.error = Some(GetLastError());
                return flow;
            }
            flow.status.applied = true;
            let mut priority = Priority::default();
            let mut size = std::mem::size_of::<Priority>() as u32;
            if (api.query)(
                handle,
                id,
                1,
                &mut size,
                &mut priority,
                0,
                std::ptr::null_mut(),
            ) != 0
            {
                flow.status.configured_dscp = Some(priority.dscp);
                flow.status.configured_l2 = Some(priority.l2);
            } else {
                flow.status.error = Some(GetLastError());
            }
        }
        flow
    }
    impl Drop for Flow {
        fn drop(&mut self) {
            if self.handle != 0 {
                if let Some(api) = api() {
                    // Closing the handle removes its flow; the socket stays owned by its stream.
                    unsafe {
                        (api.close)(self.handle as Handle);
                    }
                }
            }
        }
    }
}
