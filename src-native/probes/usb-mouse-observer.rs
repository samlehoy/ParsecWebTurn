//! Read-only host-side observation of the enumeration probe. No private IOCTLs,
//! input registration, capture, injection or administrator privileges.
use std::{
    fs::File,
    io::{self, Write},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Waiting,
    Fast,
    Quiet,
    Slow,
}
impl Phase {
    fn at(arrival_age: Option<Duration>) -> Self {
        match arrival_age {
            None => Self::Waiting,
            Some(age) if age < Duration::from_secs(3) => Self::Fast,
            Some(age) if age < Duration::from_secs(7) => Self::Quiet,
            Some(_) => Self::Slow,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Fast => "fast_100ms",
            Self::Quiet => "quiet_no_device_queries",
            Self::Slow => "slow_1000ms",
        }
    }
}

#[cfg(windows)]
fn mice(mut caps_log: Option<&mut File>) -> io::Result<u32> {
    use std::{ffi::c_void, ptr};
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Device {
        handle: *mut c_void,
        kind: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Caps {
        usage: u16,
        page: u16,
        input_length: u16,
        output_length: u16,
        feature_length: u16,
        reserved: [u16; 17],
        links: u16,
        input_buttons: u16,
        input_values: u16,
        input_indices: u16,
        output_buttons: u16,
        output_values: u16,
        output_indices: u16,
        feature_buttons: u16,
        feature_values: u16,
        feature_indices: u16,
    }
    #[link(name = "hid")]
    unsafe extern "system" {
        fn HidP_GetCaps(preparsed: *const c_void, caps: *mut Caps) -> i32;
        fn HidD_GetPreparsedData(device: *mut c_void, data: *mut *mut c_void) -> u8;
        fn HidD_FreePreparsedData(data: *mut c_void) -> u8;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            sharing: u32,
            security: *const c_void,
            disposition: u32,
            flags: u32,
            template: *mut c_void,
        ) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
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
            #[repr(align(4))]
            struct Name([u16; 4096]);
            let mut name = Name([0u16; 4096]);
            let mut length = name.0.len() as u32;
            let read = GetRawInputDeviceInfoW(
                device.handle,
                0x20000007,
                name.0.as_mut_ptr().cast(),
                &mut length,
            );
            if read == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            let end = name.0.iter().position(|v| *v == 0).unwrap_or(name.0.len());
            let identity = String::from_utf16_lossy(&name.0[..end]).to_ascii_uppercase();
            if identity.contains("VID_FFFF&PID_F001") || identity.contains("VID_FFFF&PID_F002") {
                found += 1;
                if let Some(log) = caps_log.as_deref_mut() {
                    // RIM_TYPEMOUSE can return an empty RIDI_PREPARSEDDATA.
                    // Open only the matching synthetic HID collection with
                    // ZERO desired access and shared read/write for metadata.
                    // No ReadFile/WriteFile, report IO or access-right retry.
                    let handle =
                        CreateFileW(name.0.as_ptr(), 0, 3, ptr::null(), 3, 0, ptr::null_mut());
                    if handle == (-1isize) as *mut c_void {
                        writeln!(
                            log,
                            "{{\"event\":\"hid_caps_error\",\"unix_ms\":{},\"stage\":\"metadata_open\",\"os_error\":{}}}",
                            unix_ms(),
                            io::Error::last_os_error().raw_os_error().unwrap_or(0)
                        )?;
                        continue;
                    }
                    let mut data = ptr::null_mut();
                    let available =
                        HidD_GetPreparsedData(handle, &mut data) != 0 && !data.is_null();
                    let error = if available {
                        0
                    } else {
                        io::Error::last_os_error().raw_os_error().unwrap_or(0)
                    };
                    let mut caps = Caps::default();
                    let status = if available {
                        HidP_GetCaps(data, &mut caps)
                    } else {
                        0
                    };
                    // Release both resources BEFORE any fallible log writes.
                    if available {
                        let _ = HidD_FreePreparsedData(data);
                    }
                    let _ = CloseHandle(handle);
                    if !available {
                        writeln!(
                            log,
                            "{{\"event\":\"hid_caps_error\",\"unix_ms\":{},\"stage\":\"hid_preparsed\",\"os_error\":{error}}}",
                            unix_ms()
                        )?;
                        continue;
                    }
                    writeln!(
                        log,
                        "{{\"event\":\"hid_caps\",\"unix_ms\":{},\"status\":{status},\"usage_page\":{},\"usage\":{},\"input_report_bytes\":{},\"input_button_caps\":{},\"input_value_caps\":{}}}",
                        unix_ms(),
                        caps.page,
                        caps.usage,
                        caps.input_length,
                        caps.input_buttons,
                        caps.input_values
                    )?;
                    log.flush()?;
                }
            }
        }
        Ok(found)
    }
}
#[cfg(not(windows))]
fn mice(_caps_log: Option<&mut File>) -> io::Result<u32> {
    Err(io::Error::other("Windows is required"))
}

fn run() -> io::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| io::Error::other("Supply a NEW host-observer.jsonl filename"))?;
    let mut log = File::options().write(true).create_new(true).open(path)?;
    let start = Instant::now();
    let baseline = mice(None)?;
    writeln!(
        log,
        "{{\"event\":\"observer_started\",\"unix_ms\":{},\"baseline_count\":{baseline}}}",
        unix_ms()
    )?;
    println!("Watching the diagnostic mouse for 120 seconds. Connect the probe client now.");
    let mut previous = baseline;
    let mut arrived = false;
    let mut removed = false;
    let mut errors = 0;
    let mut arrival = None;
    let mut previous_phase = None;
    while start.elapsed() < Duration::from_secs(120) {
        let phase = Phase::at(arrival.map(|time: Instant| time.elapsed()));
        if previous_phase != Some(phase) {
            writeln!(
                log,
                "{{\"event\":\"phase_changed\",\"elapsed_ms\":{},\"unix_ms\":{},\"phase\":\"{}\"}}",
                start.elapsed().as_millis(),
                unix_ms(),
                phase.name()
            )?;
            log.flush()?;
            previous_phase = Some(phase);
        }
        if phase == Phase::Quiet {
            // Deliberately call no device enumeration or metadata API here.
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        match mice(None) {
            Ok(count) => {
                if count != previous {
                    arrived |= count > baseline;
                    removed |= arrived && count == baseline;
                    writeln!(
                        log,
                        "{{\"event\":\"mouse_count_changed\",\"elapsed_ms\":{},\"unix_ms\":{},\"count\":{count}}}",
                        start.elapsed().as_millis(),
                        unix_ms()
                    )?;
                    log.flush()?;
                    println!(
                        "{} ms: diagnostic mouse count = {count}",
                        start.elapsed().as_millis()
                    );
                    previous = count;
                    if count > baseline && arrival.is_none() {
                        arrival = Some(Instant::now());
                        writeln!(
                            log,
                            "{{\"event\":\"caps_check_started\",\"unix_ms\":{}}}",
                            unix_ms()
                        )?;
                        if let Err(error) = mice(Some(&mut log)) {
                            writeln!(
                                log,
                                "{{\"event\":\"hid_caps_error\",\"unix_ms\":{},\"stage\":\"enumeration\",\"os_error\":{}}}",
                                unix_ms(),
                                error.raw_os_error().unwrap_or(0)
                            )?;
                        }
                        writeln!(
                            log,
                            "{{\"event\":\"caps_check_finished\",\"unix_ms\":{}}}",
                            unix_ms()
                        )?;
                        log.flush()?;
                    }
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
        std::thread::sleep(Duration::from_millis(if phase == Phase::Slow {
            1000
        } else {
            100
        }));
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observation_has_distinct_fast_quiet_and_slow_phases() {
        assert_eq!(Phase::at(None), Phase::Waiting);
        assert_eq!(Phase::at(Some(Duration::from_millis(2999))), Phase::Fast);
        assert_eq!(Phase::at(Some(Duration::from_secs(3))), Phase::Quiet);
        assert_eq!(Phase::at(Some(Duration::from_millis(6999))), Phase::Quiet);
        assert_eq!(Phase::at(Some(Duration::from_secs(7))), Phase::Slow);
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
