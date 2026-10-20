//! ParsecWebTurn-owned interoperability diagnostic. Never compiled by default.
//! Fixed desktop input sequence requires an additional explicit runtime opt-in.
//! Private wire observations: docs/usb-native-wire-investigation.md.
#![allow(
    clippy::indexing_slicing,
    reason = "fixed local records; peer records are length checked"
)]

use std::{
    fs::File,
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const BODY: usize = 1932;
pub(crate) const FRAME: usize = BODY + 13;
const RECORD: usize = 1570;
// Unassigned diagnostic identity, NOT an assigned vendor or a security key.
const ID: u32 = 0xffff_f001;
static ARMED: AtomicBool = AtomicBool::new(false);
const REPORT: &[u8] = &[
    0x05, 1, 0x09, 2, 0xa1, 1, 0x09, 1, 0xa1, 0, 0x05, 9, 0x19, 1, 0x29, 3, 0x15, 0, 0x25, 1, 0x95,
    3, 0x75, 1, 0x81, 2, 0x95, 1, 0x75, 5, 0x81, 3, 0x05, 1, 0x09, 0x30, 0x09, 0x31, 0x15, 0x81,
    0x25, 0x7f, 0x75, 8, 0x95, 2, 0x81, 6, 0xc0, 0xc0,
];
const DEVICE: &[u8] = &[
    18, 1, 0, 2, 0, 0, 0, 64, 0xff, 0xff, 1, 0xf0, 0, 1, 1, 2, 3, 1,
];
const CONFIG: &[u8] = &[
    9, 2, 34, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 1, 3, 1, 2, 0, 9, 0x21, 0x11, 1, 0, 1, 0x22, 50, 0,
    7, 5, 0x81, 3, 3, 0, 10,
];
const MAKER: &str = "ParsecWebTurn research";
const PRODUCT: &str = "USB mouse enumeration probe";

#[path = "usb_mouse_absolute_probe.rs"]
mod absolute_probe;
#[path = "usb_mouse_input_probe.rs"]
mod input_probe;
#[path = "usb_mouse_live_probe.rs"]
mod live_probe;

pub(crate) fn live_requested() -> bool {
    std::env::var("PARSEC_USB_MOUSE_PROBE").as_deref() == Ok("1")
        && std::env::var("PARSEC_USB_MOUSE_LIVE_PROBE").as_deref() == Ok("1")
}
pub(crate) fn absolute_requested() -> bool {
    std::env::var("PARSEC_USB_MOUSE_PROBE").as_deref() == Ok("1")
        && std::env::var("PARSEC_USB_MOUSE_ABSOLUTE_PROBE").as_deref() == Ok("1")
}

pub(crate) fn mouse_opcode(opcode: u8) -> bool {
    matches!(opcode, 1 | 2 | 3 | 26)
}

fn put(record: &mut [u8], offset: usize, value: u32) {
    record[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn word(record: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([record[offset], record[offset + 1]])
}
fn dword(record: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(record[offset..offset + 4].try_into().unwrap_or_default())
}
fn frame(record: &[u8; RECORD]) -> [u8; FRAME] {
    let mut out = [0; FRAME];
    out[..4].copy_from_slice(&1932u32.to_be_bytes());
    out[4..8].copy_from_slice(&1u32.to_be_bytes());
    out[8..12].copy_from_slice(&ID.to_be_bytes());
    out[12] = 32;
    put(&mut out, 13, 14);
    put(&mut out, 17, 1570);
    out[21..21 + RECORD].copy_from_slice(record);
    out
}
fn identity() -> [u8; FRAME] {
    let mut r = [0; RECORD];
    put(&mut r, 0, 15);
    r[4..22].copy_from_slice(DEVICE);
    r[22..22 + MAKER.len()].copy_from_slice(MAKER.as_bytes());
    r[277..277 + PRODUCT.len()].copy_from_slice(PRODUCT.as_bytes());
    r[532..536].copy_from_slice(&[4, 3, 9, 4]);
    r[542..542 + CONFIG.len()].copy_from_slice(CONFIG);
    put(&mut r, 1566, 34);
    frame(&r)
}
fn configuration() -> [u8; FRAME] {
    configuration_for(CONFIG)
}
fn configuration_for(config: &[u8]) -> [u8; FRAME] {
    let mut r = [0; RECORD];
    put(&mut r, 0, 11);
    r[8..17].copy_from_slice(&config[..9]);
    r[17..26].copy_from_slice(&config[9..18]);
    r[26..33].copy_from_slice(&config[27..34]);
    put(&mut r, 17 + 180, 1); // one alternate, one interface, one endpoint
    frame(&r)
}
fn string(value: &str) -> Vec<u8> {
    let mut bytes = vec![0, 3];
    for ch in value.encode_utf16() {
        bytes.extend_from_slice(&ch.to_le_bytes());
    }
    bytes[0] = u8::try_from(bytes.len()).unwrap_or(0);
    bytes
}

/// Only the fixed device's enumeration requests are implemented. Unsupported
/// requests stall (-9, LIBUSB_ERROR_PIPE), never receive an invented success.
fn control(r: &[u8]) -> ([u8; FRAME], bool) {
    control_for(r, None)
}
fn control_for(r: &[u8], live_buttons: Option<u8>) -> ([u8; FRAME], bool) {
    control_layout(r, live_buttons, false)
}
fn control_layout(r: &[u8], live_buttons: Option<u8>, absolute: bool) -> ([u8; FRAME], bool) {
    let (device, config, report) = if absolute {
        (
            absolute_probe::DEVICE,
            absolute_probe::CONFIG,
            absolute_probe::REPORT,
        )
    } else if live_buttons.is_some() {
        (live_probe::DEVICE, live_probe::CONFIG, live_probe::REPORT)
    } else {
        (DEVICE, CONFIG, REPORT)
    };
    let mut reply = [0; RECORD];
    put(&mut reply, 0, 10);
    put(&mut reply, 1036, dword(r, 1042));
    let length = usize::from(word(r, 1034));
    let value = word(r, 6);
    let index = word(r, 8);
    let data = match (r[4], r[5], value, index) {
        (0x80, 6, 0x100, 0) => Some(device.to_vec()),
        (0x80, 6, 0x200, 0) => Some(config.to_vec()),
        (0x80, 6, 0x300, _) => Some(vec![4, 3, 9, 4]),
        (0x80, 6, 0x301, 0x409) => Some(string(MAKER)),
        (0x80, 6, 0x302, 0x409) => Some(string(PRODUCT)),
        (0x80, 6, 0x303, 0x409) => Some(string(if absolute {
            "ABS-GRID"
        } else if live_buttons.is_some() {
            "LIVE-TEST"
        } else {
            "ENUM-ONLY"
        })),
        (0x81, 6, 0x2100, 0) => Some(config[18..27].to_vec()),
        (0x81, 6, 0x2200, 0) => Some(report.to_vec()),
        (0x80, 8, 0, 0) => Some(vec![1]),  // GET_CONFIGURATION
        (0x81, 10, 0, 0) => Some(vec![0]), // GET_INTERFACE
        (0x80..=0x82, 0, 0, _) => Some(vec![0, 0]), // GET_STATUS
        (0x00, 9, 1, 0) | (0x01, 11, 0, 0) => Some(vec![]),
        // No fabricated current absolute position before a real input report.
        (0xa1, 1, 0x100, 0) if !absolute => {
            Some(live_buttons.map_or_else(|| vec![0; 3], |b| vec![b, 0, 0, 0, 0]))
        }
        (0xa1, 2, 0, 0) => Some(vec![0]),
        (0xa1, 3, 0, 0) => Some(vec![1]),
        (0x21, 10, _, 0) | (0x21, 11, 0..=1, 0) => Some(vec![]),
        _ => None,
    };
    let supported = length <= 1024 && data.is_some();
    if let Some(data) = data.filter(|_| supported) {
        let count = data.len().min(length);
        reply[8..8 + count].copy_from_slice(&data[..count]);
        // Native libusb_control_transfer returns the transferred count in BOTH
        // the status and length fields (negative status only denotes failure).
        put(&mut reply, 4, u32::try_from(count).unwrap_or(0));
        put(&mut reply, 1032, u32::try_from(count).unwrap_or(0));
    } else {
        put(&mut reply, 4, (-9i32) as u32);
    }
    (frame(&reply), supported)
}

#[derive(Debug)]
pub(crate) struct Probe {
    log: File,
    start: Option<f64>,
    announced: bool,
    removed: bool,
    received: u32,
    elapsed_ms: u32,
    input: Option<input_probe::Input>,
    stopping: bool,
    live: Option<live_probe::Input>,
    absolute: Option<absolute_probe::Input>,
    next_live_log: u32,
}
impl Probe {
    pub(crate) fn from_environment() -> Option<Self> {
        if std::env::var("PARSEC_USB_MOUSE_PROBE").as_deref() != Ok("1") {
            return None;
        }
        let path = std::env::var_os("PARSEC_USB_PROBE_LOG")?;
        // A writable dedicated log is mandatory before any network mutation.
        let log = File::options()
            .write(true)
            .create_new(true)
            .open(path)
            .ok()?;
        if ARMED.swap(true, Ordering::AcqRel) {
            return None;
        }
        let mut probe = Self {
            log,
            start: None,
            announced: false,
            removed: false,
            received: 0,
            elapsed_ms: 0,
            input: (!live_requested()
                && !absolute_requested()
                && std::env::var("PARSEC_USB_MOUSE_INPUT_PROBE").as_deref() == Ok("1"))
            .then(input_probe::Input::default),
            stopping: false,
            live: (live_requested() && !absolute_requested()).then(live_probe::Input::default),
            absolute: absolute_requested().then(absolute_probe::Input::default),
            next_live_log: 0,
        };
        probe.note("armed", 0, 0, 0);
        if probe.input.is_some() {
            probe.note("fixed_input_enabled", 8, 0, 0);
        }
        if probe.live.is_some() {
            probe.note("live_hid_enabled_normal_mouse_blocked", 8, 0, 0);
        }
        if probe.absolute.is_some() {
            probe.note("absolute_grid_enabled_normal_mouse_blocked", 8, 9, 0);
        }
        Some(probe)
    }
    fn note(&mut self, event: &str, kind: u32, code: u32, detail: u32) {
        // Only fixed event names and integers; no account, peer or USB payloads.
        let _ = writeln!(
            self.log,
            "{{\"event\":\"{event}\",\"elapsed_ms\":{},\"unix_ms\":{},\"kind\":{kind},\"code\":{code},\"detail\":{detail}}}",
            self.elapsed_ms,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let _ = self.log.flush();
    }
    fn exchange(&mut self, request: &[u8], response: &[u8; FRAME]) {
        let reply = &response[21..21 + RECORD];
        // Setup fields and correlation only; never record arbitrary payloads.
        let _ = writeln!(
            self.log,
            "{{\"event\":\"control_exchange\",\"elapsed_ms\":{},\"unix_ms\":{},\"request_type\":{},\"request\":{},\"value\":{},\"index\":{},\"requested_length\":{},\"transfer_capacity\":{},\"correlation_id\":{},\"reply_correlation_id\":{},\"reply_length\":{},\"status\":{}}}",
            self.elapsed_ms,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            request[4],
            request[5],
            word(request, 6),
            word(request, 8),
            word(request, 1034),
            word(request, 1040),
            dword(request, 1042),
            dword(reply, 1036),
            dword(reply, 1032),
            dword(reply, 4) as i32
        );
        let _ = self.log.flush();
    }
    pub(crate) fn tick(&mut self, now: f64) -> Option<[u8; FRAME]> {
        let start = *self.start.get_or_insert(now);
        self.elapsed_ms = (now - start).clamp(0.0, f64::from(u32::MAX)) as u32;
        if self.removed {
            return None;
        }
        if self.stopping {
            return self.remove();
        }
        if now - start
            >= if self.live.is_some() {
                90_000.0
            } else {
                17_000.0
            }
        {
            if let Some(release) = self.release_input() {
                return Some(release);
            }
            return self.remove();
        }
        if !self.announced && now - start >= 2_000.0 {
            self.announced = true;
            self.note("identity_queued", 15, 0, 0);
            let mut packet = identity();
            if self.live.is_some() {
                packet[25..43].copy_from_slice(live_probe::DEVICE);
                packet[21 + 542..21 + 542 + 34].copy_from_slice(live_probe::CONFIG);
            } else if self.absolute.is_some() {
                packet[25..43].copy_from_slice(absolute_probe::DEVICE);
                packet[21 + 542..21 + 542 + 34].copy_from_slice(absolute_probe::CONFIG);
            }
            return Some(packet);
        }
        if self.live.is_some() {
            let input = self.live.as_mut()?;
            if input.failed() {
                self.note("live_hid_backlog_limit", 8, 0, 0);
                return self.remove();
            }
            let report = input.next(self.elapsed_ms);
            if self.elapsed_ms >= self.next_live_log {
                let (reports, commands, suppressed) =
                    (input.reports, input.commands, input.suppressed);
                self.note("live_hid_reports", 8, reports, commands);
                self.note("normal_mouse_commands_suppressed", 0, suppressed, 0);
                self.next_live_log = self.elapsed_ms.saturating_add(1000);
            }
            return report;
        }
        if let Some((report, step)) = self.input.as_mut().and_then(|i| i.next(self.elapsed_ms)) {
            self.note("fixed_report_queued", 8, step as u32, 0);
            return Some(report);
        }
        if let Some((report, step)) = self.absolute.as_mut().and_then(|i| i.next(self.elapsed_ms)) {
            self.note("absolute_report_queued", 8, step as u32, 0);
            return Some(report);
        }
        None
    }
    pub(crate) fn incoming(&mut self, bytes: &[u8]) -> Option<[u8; FRAME]> {
        if !self.announced
            || self.removed
            || bytes.len() != FRAME
            || bytes[12] != 32
            || bytes[..4] != 1932u32.to_be_bytes()
            || bytes[8..12]
                != (if self.live.is_some() {
                    live_probe::ID
                } else if self.absolute.is_some() {
                    absolute_probe::ID
                } else {
                    ID
                })
                .to_be_bytes()
            || dword(bytes, 13) != 14
            || dword(bytes, 17) != 1570
        {
            return None;
        }
        self.received += 1;
        if self.received > if self.live.is_some() { 32768 } else { 512 } {
            return self.remove();
        }
        let r = &bytes[21..21 + RECORD];
        let kind = dword(r, 0);
        self.note("host_request", kind, u32::from(r[5]), u32::from(word(r, 6)));
        match kind {
            2 => Some(if self.live.is_some() {
                configuration_for(live_probe::CONFIG)
            } else if self.absolute.is_some() {
                configuration_for(absolute_probe::CONFIG)
            } else {
                configuration()
            }),
            1 => {
                let (reply, supported) = if self.absolute.is_some() {
                    control_layout(r, None, true)
                } else if let Some(live) = &self.live {
                    control_for(r, Some(live.buttons()))
                } else {
                    control(r)
                };
                self.exchange(r, &reply);
                self.note(
                    if supported {
                        "control_reply"
                    } else {
                        "control_stalled"
                    },
                    10,
                    u32::from(r[5]),
                    u32::from(word(r, 6)),
                );
                Some(reply)
            }
            7 => {
                if let Some(absolute) = &mut self.absolute {
                    if !absolute.accept(r, self.elapsed_ms) {
                        self.note("interrupt_rejected", 7, dword(r, 8), dword(r, 16));
                        return self.remove();
                    }
                    return None;
                }
                if let Some(live) = &mut self.live {
                    if !live.accept(r) {
                        self.note("interrupt_rejected", 7, dword(r, 8), dword(r, 16));
                        return self.remove();
                    }
                    return None;
                }
                if let Some(input) = &mut self.input {
                    if !input.accept(r, self.elapsed_ms) {
                        self.note("interrupt_rejected", 7, 0, 0);
                        return self.remove();
                    }
                    self.note("interrupt_read_pending", 7, dword(r, 8), dword(r, 16));
                }
                None
            }
            _ => self.remove(),
        }
    }
    pub(crate) fn release_input(&mut self) -> Option<[u8; FRAME]> {
        let release = if let Some(live) = &mut self.live {
            live.release()
        } else {
            self.input.as_mut().and_then(input_probe::Input::release)
        };
        if release.is_some() {
            self.note("button_release_queued", 8, 0, 0);
        }
        release
    }
    pub(crate) fn external_mouse(&mut self, opcode: u8, a: u32, b: u32, c: u32) {
        if !self.announced || self.stopping || self.removed {
            return;
        }
        if let Some(live) = &mut self.live {
            live.command(opcode, a, b, c);
        }
    }
    pub(crate) fn live_active(&self) -> bool {
        self.announced
            && !self.stopping
            && !self.removed
            && self.live.as_ref().is_some_and(live_probe::Input::ready)
    }
    pub(crate) fn prepare_frame(&self, packet: &mut [u8; FRAME]) {
        if self.live.is_some() {
            packet[8..12].copy_from_slice(&live_probe::ID.to_be_bytes());
        } else if self.absolute.is_some() {
            packet[8..12].copy_from_slice(&absolute_probe::ID.to_be_bytes());
        }
    }
    pub(crate) fn remove(&mut self) -> Option<[u8; FRAME]> {
        if !self.announced || self.removed {
            return None;
        }
        self.stopping = true;
        if let Some(release) = self.release_input() {
            return Some(release);
        }
        self.removed = true;
        if let Some(live) = &self.live {
            let (reports, commands, suppressed) = (live.reports, live.commands, live.suppressed);
            self.note("live_hid_final", 8, reports, commands);
            self.note("normal_mouse_commands_suppressed", 0, suppressed, 0);
        }
        self.note("remove_queued", 16, self.received, 0);
        let mut r = [0; RECORD];
        put(&mut r, 0, 16);
        Some(frame(&r))
    }
    pub(crate) fn sent(&mut self, success: bool) {
        if !success {
            self.stopping = true;
        }
        self.note(
            if success {
                "transport_accepted"
            } else {
                "transport_refused"
            },
            0,
            0,
            0,
        );
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        self.note(
            if self.announced && !self.removed {
                "session_ended_before_remove"
            } else {
                "probe_finished"
            },
            0,
            self.received,
            0,
        );
    }
}

#[cfg(test)]
#[path = "usb_mouse_interrupt_fixture.rs"]
mod interrupt_contract;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usb_mouse_probe_absolute_descriptor_and_reply_are_consistent() {
        assert_eq!(word(absolute_probe::DEVICE, 10), 0xf003);
        assert_eq!(word(absolute_probe::CONFIG, 31), 5);
        assert_eq!(
            usize::from(word(absolute_probe::CONFIG, 25)),
            absolute_probe::REPORT.len()
        );
        let mut request = [0; RECORD];
        request[4] = 0x81;
        request[5] = 6;
        request[6..8].copy_from_slice(&0x2200u16.to_le_bytes());
        request[1034..1036].copy_from_slice(&128u16.to_le_bytes());
        let (wire, supported) = control_layout(&request, None, true);
        assert!(supported);
        assert_eq!(
            dword(&wire, 21 + 1032) as usize,
            absolute_probe::REPORT.len()
        );
        assert_eq!(
            &wire[29..29 + absolute_probe::REPORT.len()],
            absolute_probe::REPORT
        );
        let report = absolute_probe::REPORT;
        let (mut at, mut size, mut count, mut bits, mut axes_absolute) = (0, 0, 0, 0, false);
        while at < report.len() {
            let tag = report[at];
            let len = match tag & 3 {
                3 => 4,
                n => n as usize,
            };
            let value = report.get(at + 1).copied().unwrap_or(0);
            match tag {
                0x75 => size = u32::from(value),
                0x95 => count = u32::from(value),
                0x81 => {
                    bits += size * count;
                    if size == 16 && count == 2 {
                        axes_absolute = value == 2;
                    }
                }
                _ => {}
            }
            at += 1 + len;
        }
        assert_eq!(bits, 40);
        assert!(axes_absolute);
    }
    #[test]
    fn live_usb_mouse_descriptor_contract_is_distinct_and_consistent() {
        assert_eq!(live_probe::REPORT.len(), 65);
        assert_eq!(word(live_probe::CONFIG, 25), 65);
        assert_eq!(word(live_probe::CONFIG, 31), 5);
        assert_eq!(&live_probe::CONFIG[15..17], &[0, 0]);
        assert_eq!(word(live_probe::DEVICE, 10), 0xf002);
        let mut request = [0; RECORD];
        request[4] = 0x81;
        request[5] = 6;
        request[6..8].copy_from_slice(&0x2200u16.to_le_bytes());
        request[1034..1036].copy_from_slice(&129u16.to_le_bytes());
        let (reply, supported) = control_for(&request, Some(0));
        assert!(supported);
        assert_eq!(dword(&reply, 21 + 1032), 65);
        assert_eq!(&reply[29..29 + 65], live_probe::REPORT);
        let (mut at, mut size, mut count, mut bits) = (0, 0, 0, 0);
        while at < live_probe::REPORT.len() {
            let tag = live_probe::REPORT[at];
            let len = match tag & 3 {
                3 => 4,
                n => n as usize,
            };
            let value = live_probe::REPORT.get(at + 1).copied().unwrap_or(0);
            match tag {
                0x75 => size = u32::from(value),
                0x95 => count = u32::from(value),
                0x81 => bits += size * count,
                _ => {}
            }
            at += 1 + len;
        }
        assert_eq!(bits, 40);
    }
    #[test]
    fn descriptor_lengths_and_identity_are_consistent() {
        assert_eq!(REPORT.len(), 50);
        assert_eq!(CONFIG.len(), usize::from(word(CONFIG, 2)));
        assert_eq!(usize::from(word(CONFIG, 25)), REPORT.len());
        let f = identity();
        assert_eq!(&f[8..12], &ID.to_be_bytes());
        assert_eq!(&f[21 + 4..21 + 22], DEVICE);
        assert_eq!(dword(&f, 21 + 1566), 34);
        assert_eq!(&configuration()[21 + 26..21 + 33], &CONFIG[27..34]);
    }
    #[test]
    fn report_descriptor_response_is_bounded_and_correlated() {
        let mut r = [0; RECORD];
        r[4] = 0x81;
        r[5] = 6;
        r[6..8].copy_from_slice(&0x2200u16.to_le_bytes());
        r[1034..1036].copy_from_slice(&9u16.to_le_bytes());
        put(&mut r, 1042, 0x12345678);
        let (f, ok) = control(&r);
        assert!(ok);
        assert_eq!(&f[29..38], &REPORT[..9]);
        assert_eq!(dword(&f, 21 + 4), 9);
        assert_eq!(dword(&f, 21 + 1032), 9);
        assert_eq!(dword(&f, 21 + 1036), 0x12345678);
        r[1034..1036].copy_from_slice(&1025u16.to_le_bytes());
        let (f, ok) = control(&r);
        assert!(!ok);
        assert_eq!(dword(&f, 21 + 4), (-9i32) as u32);
        assert_eq!(dword(&f, 21 + 1032), 0);
    }
    #[test]
    fn unknown_requests_stall_instead_of_succeeding() {
        let mut r = [0; RECORD];
        r[4] = 0xc0;
        r[5] = 0xff;
        assert!(!control(&r).1);
    }

    #[test]
    fn complete_report_descriptor_stays_identical_for_repeated_requests() {
        let mut r = [0; RECORD];
        r[4] = 0x81;
        r[5] = 6;
        r[6..8].copy_from_slice(&0x2200u16.to_le_bytes());
        // USB wLength and the carrier's transfer capacity are separate fields.
        r[1034..1036].copy_from_slice(&64u16.to_le_bytes());
        r[1040..1042].copy_from_slice(&1024u16.to_le_bytes());
        for id in [1, 2, 0x10203040] {
            put(&mut r, 1042, id);
            let (f, supported) = control(&r);
            assert!(supported);
            assert_eq!(dword(&f, 21 + 4), 50);
            assert_eq!(dword(&f, 21 + 1032), 50);
            assert_eq!(dword(&f, 21 + 1036), id);
            assert_eq!(&f[29..79], REPORT);
            assert!(f[79..21 + 1032].iter().all(|byte| *byte == 0));
        }
    }

    #[test]
    fn lifecycle_is_bounded_passive_and_removes_once() {
        let path = std::env::temp_dir().join(format!("usb-enum-test-{}.jsonl", std::process::id()));
        {
            let mut probe = Probe {
                log: File::create(&path).unwrap(),
                start: None,
                announced: false,
                removed: false,
                received: 0,
                elapsed_ms: 0,
                input: None,
                stopping: false,
                live: None,
                absolute: None,
                next_live_log: 0,
            };
            assert!(probe.tick(0.0).is_none());
            assert!(probe.tick(1999.0).is_none());
            assert_eq!(probe.tick(2000.0), Some(identity()));
            let mut r = [0; RECORD];
            put(&mut r, 0, 2);
            assert_eq!(probe.incoming(&frame(&r)), Some(configuration()));
            for len in 0..FRAME {
                assert!(probe.incoming(&vec![0; len]).is_none());
            }
            put(&mut r, 0, 7);
            assert!(probe.incoming(&frame(&r)).is_none());
            let removed = probe.tick(17_000.0).unwrap();
            assert_eq!(dword(&removed, 21), 16);
            assert!(probe.remove().is_none());
            assert!(probe.incoming(&frame(&r)).is_none());
        }
        std::fs::remove_file(path).unwrap();
    }
}
