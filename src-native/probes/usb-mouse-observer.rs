//! Read-only host-side observation of the enumeration probe. No driver calls,
//! input registration, capture, injection or administrator privileges.
use std::{
    fs::File,
    io::{self, Write},
    time::{Duration, Instant},
};

#[cfg(windows)]
fn mice() -> io::Result<u32> {
    use std::{ffi::c_void, ptr};
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Device {
        handle: *mut c_void,
        kind: u32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetRawInputDeviceList(devices: *mut Device, count: *mut u32, size: u32) -> u32;
        fn GetRawInputDeviceInfoW(
            device: *mut c_void,
            command: u32,
            data: *mut c_void,
            count: *mut u32,
        ) -> u32;
    }
    let size = size_of::<Device>() as u32;
    let mut count = 0;
    // Documented Win32 metadata enumeration; no input reports are read.
    unsafe {
        if GetRawInputDeviceList(ptr::null_mut(), &mut count, size) == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        if count > 512 {
            return Err(io::Error::other("device count exceeds diagnostic bound"));
        }
        if count == 0 {
            return Ok(0);
        }
        let mut devices = vec![
            Device {
                handle: ptr::null_mut(),
                kind: 0
            };
            count as usize
        ];
        let read = GetRawInputDeviceList(devices.as_mut_ptr(), &mut count, size);
        if read == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        let mut found = 0;
        for device in devices.iter().take(read as usize).filter(|d| d.kind == 0) {
            let mut name = [0u16; 4096];
            let mut length = name.len() as u32;
            let read = GetRawInputDeviceInfoW(
                device.handle,
                0x20000007,
                name.as_mut_ptr().cast(),
                &mut length,
            );
            if read == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            let end = name.iter().position(|v| *v == 0).unwrap_or(name.len());
            if String::from_utf16_lossy(&name[..end])
                .to_ascii_uppercase()
                .contains("VID_FFFF&PID_F001")
            {
                found += 1;
            }
        }
        Ok(found)
    }
}
#[cfg(not(windows))]
fn mice() -> io::Result<u32> {
    Err(io::Error::other("Windows is required"))
}

fn run() -> io::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| io::Error::other("Supply a NEW host-observer.jsonl filename"))?;
    let mut log = File::options().write(true).create_new(true).open(path)?;
    let start = Instant::now();
    let baseline = mice()?;
    writeln!(
        log,
        "{{\"event\":\"observer_started\",\"baseline_count\":{baseline}}}"
    )?;
    println!("Watching the diagnostic mouse for 120 seconds. Connect the probe client now.");
    let mut previous = baseline;
    let mut arrived = false;
    let mut removed = false;
    let mut errors = 0;
    while start.elapsed() < Duration::from_secs(120) {
        match mice() {
            Ok(count) => {
                if count != previous {
                    arrived |= count > baseline;
                    removed |= arrived && count == baseline;
                    writeln!(
                        log,
                        "{{\"event\":\"mouse_count_changed\",\"elapsed_ms\":{},\"count\":{count}}}",
                        start.elapsed().as_millis()
                    )?;
                    log.flush()?;
                    println!(
                        "{} ms: diagnostic mouse count = {count}",
                        start.elapsed().as_millis()
                    );
                    previous = count;
                }
                if removed {
                    break;
                }
            }
            Err(error) => {
                errors += 1;
                writeln!(
                    log,
                    "{{\"event\":\"observation_error\",\"elapsed_ms\":{},\"os_error\":{}}}",
                    start.elapsed().as_millis(),
                    error.raw_os_error().unwrap_or(0)
                )?;
                if errors >= 10 {
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    writeln!(
        log,
        "{{\"event\":\"observer_finished\",\"arrival_observed\":{arrived},\"removal_observed\":{removed},\"errors\":{errors}}}"
    )?;
    println!(
        "Arrival observed: {arrived}; removal observed: {removed}; observation errors: {errors}"
    );
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
