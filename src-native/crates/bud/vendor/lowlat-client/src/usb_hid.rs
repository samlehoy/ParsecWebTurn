//! Shared optional USB HID device lifecycle; independent of video decode.
#![allow(
    clippy::indexing_slicing,
    reason = "fixed local records; peer records are length checked"
)]
#[path = "hid_mouse_input.rs"]
mod input;
#[path = "hid_keyboard_input.rs"]
mod keyboard;
pub(crate) const FRAME: usize = 1945;
const RECORD: usize = 1570;
const MAKER: &str = "ParsecWebTurn";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Mouse,
    Keyboard,
}
impl Kind {
    pub(crate) const ALL: [Self; 2] = [Self::Mouse, Self::Keyboard];
}
struct Layout {
    id: u32,
    device: &'static [u8],
    config: &'static [u8],
    report: &'static [u8],
    product: &'static str,
    serial: &'static str,
}
impl Kind {
    fn layout(self) -> Layout {
        match self {
            Self::Mouse => Layout {
                id: input::ID,
                device: input::DEVICE,
                config: input::CONFIG,
                report: input::REPORT,
                product: "Experimental USB HID mouse",
                serial: "ABS-LIVE",
            },
            Self::Keyboard => Layout {
                id: keyboard::ID,
                device: keyboard::DEVICE,
                config: keyboard::CONFIG,
                report: keyboard::REPORT,
                product: "Experimental USB HID keyboard",
                serial: "KEYBOARD",
            },
        }
    }
}
#[derive(Debug)]
enum Input {
    Mouse(input::Input),
    Keyboard(keyboard::Input),
}
impl Default for Input {
    fn default() -> Self {
        Self::Mouse(input::Input::default())
    }
}
// The lifecycle calls the same small report interface on either device.
macro_rules! forward {
    ($($name:ident($($arg:ident:$ty:ty),*) -> $ret:ty;)*) => {$ (
        fn $name(&mut self, $($arg:$ty),*) -> $ret {
            match self {
                Self::Mouse(v) => v.$name($($arg),*),
                Self::Keyboard(v) => v.$name($($arg),*),
            }
        }
    )*};
}
impl Input {
    fn kind(&self) -> Kind {
        match self {
            Self::Mouse(_) => Kind::Mouse,
            Self::Keyboard(_) => Kind::Keyboard,
        }
    }
    forward! {
        ready() -> bool;
        failed() -> bool;
        accept(r: &[u8]) -> bool;
        next(now: u64) -> Option<[u8; FRAME]>;
        release() -> Option<[u8; FRAME]>;
        command(op: u8, a: u32, b: u32, c: u32) -> ();
    }
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
fn frame(record: &[u8; RECORD], id: u32) -> [u8; FRAME] {
    let mut out = [0; FRAME];
    out[..4].copy_from_slice(&1932u32.to_be_bytes());
    out[4..8].copy_from_slice(&1u32.to_be_bytes());
    out[8..12].copy_from_slice(&id.to_be_bytes());
    out[12] = 32;
    put(&mut out, 13, 14);
    put(&mut out, 17, 1570);
    out[21..21 + RECORD].copy_from_slice(record);
    out
}
fn identity(layout: &Layout) -> [u8; FRAME] {
    let mut r = [0; RECORD];
    put(&mut r, 0, 15);
    r[4..22].copy_from_slice(layout.device);
    r[22..22 + MAKER.len()].copy_from_slice(MAKER.as_bytes());
    r[277..277 + layout.product.len()].copy_from_slice(layout.product.as_bytes());
    r[532..536].copy_from_slice(&[4, 3, 9, 4]);
    r[542..542 + layout.config.len()].copy_from_slice(layout.config);
    put(&mut r, 1566, 34);
    frame(&r, layout.id)
}
fn configuration_for(config: &[u8], id: u32) -> [u8; FRAME] {
    let mut r = [0; RECORD];
    put(&mut r, 0, 11);
    r[8..17].copy_from_slice(&config[..9]);
    r[17..26].copy_from_slice(&config[9..18]);
    r[26..33].copy_from_slice(&config[27..34]);
    put(&mut r, 17 + 180, 1); // one alternate, one interface, one endpoint
    frame(&r, id)
}
fn string(value: &str) -> Vec<u8> {
    let mut bytes = vec![0, 3];
    for ch in value.encode_utf16() {
        bytes.extend_from_slice(&ch.to_le_bytes());
    }
    bytes[0] = u8::try_from(bytes.len()).unwrap_or(0);
    bytes
}

fn control(r: &[u8], layout: &Layout, input: &mut Input) -> [u8; FRAME] {
    let (device, config, report) = (layout.device, layout.config, layout.report);
    let mut reply = [0; RECORD];
    put(&mut reply, 0, 10);
    put(&mut reply, 1036, dword(r, 1042));
    let length = usize::from(word(r, 1034));
    let value = word(r, 6);
    let index = word(r, 8);
    let request = (r[4], r[5], value, index);
    let keyboard_reply = if let Input::Keyboard(k) = input {
        match request {
            (0xa1, 1, 0x100, 0) => Some(Some(k.report().to_vec())),
            (0xa1, 1, 0x200, 0) => Some(Some(vec![k.leds])),
            (0x21, 9, 0x200, 0) => {
                if length == 1 {
                    k.leds = r[10] & 0x1f;
                    Some(Some(vec![r[10]]))
                } else {
                    Some(None)
                }
            }
            (0xa1, 2, 0, 0) => Some(Some(vec![k.idle])),
            (0x21, 10, _, 0) if value & 255 == 0 && length == 0 => {
                k.idle = (value >> 8) as u8;
                Some(Some(vec![]))
            }
            // This descriptor offers report protocol only, never boot protocol.
            (0x21, 11, 0, 0) => Some(None),
            _ => None,
        }
    } else {
        None
    };
    let data = keyboard_reply.unwrap_or_else(|| match request {
        (0x80, 6, 0x100, 0) => Some(device.to_vec()),
        (0x80, 6, 0x200, 0) => Some(config.to_vec()),
        (0x80, 6, 0x300, _) => Some(vec![4, 3, 9, 4]),
        (0x80, 6, 0x301, 0x409) => Some(string(MAKER)),
        (0x80, 6, 0x302, 0x409) => Some(string(layout.product)),
        (0x80, 6, 0x303, 0x409) => Some(string(layout.serial)),
        (0x81, 6, 0x2100, 0) => Some(config[18..27].to_vec()),
        (0x81, 6, 0x2200, 0) => Some(report.to_vec()),
        (0x80, 8, 0, 0) => Some(vec![1]),  // GET_CONFIGURATION
        (0x81, 10, 0, 0) => Some(vec![0]), // GET_INTERFACE
        (0x80..=0x82, 0, 0, _) => Some(vec![0, 0]), // GET_STATUS
        (0x00, 9, 1, 0) | (0x01, 11, 0, 0) => Some(vec![]),
        (0xa1, 2, 0, 0) => Some(vec![0]),
        (0xa1, 3, 0, 0) => Some(vec![1]),
        (0x21, 10, _, 0) | (0x21, 11, 0..=1, 0) => Some(vec![]),
        _ => None,
    });
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
    frame(&reply, layout.id)
}

#[derive(Debug, Default)]
pub(crate) struct Device {
    start: Option<f64>,
    announced: bool,
    removed: bool,
    error: u32,
    elapsed: u64,
    input: Input,
}
impl Device {
    pub(crate) fn new(kind: Kind) -> Self {
        Self {
            input: match kind {
                Kind::Mouse => Input::Mouse(input::Input::default()),
                Kind::Keyboard => Input::Keyboard(keyboard::Input::default()),
            },
            ..Self::default()
        }
    }
    pub(crate) fn tick(&mut self, now: f64) -> Option<[u8; FRAME]> {
        let start = *self.start.get_or_insert(now);
        self.elapsed = (now - start).max(0.0) as u64;
        if self.removed {
            return None;
        }
        if self.input.failed() {
            self.fail(3);
        }
        if self.announced && !self.input.ready() && self.elapsed >= 12000 {
            self.fail(1);
        }
        if self.error != 0 {
            return self.remove();
        }
        if !self.announced && self.elapsed >= 2000 {
            self.announced = true;
            return Some(identity(&self.input.kind().layout()));
        }
        self.input.next(self.elapsed)
    }
    pub(crate) fn incoming(&mut self, bytes: &[u8]) -> Option<[u8; FRAME]> {
        if !self.announced
            || self.removed
            || bytes.len() != FRAME
            || bytes[12] != 32
            || bytes[..4] != 1932u32.to_be_bytes()
            || bytes[8..12] != self.input.kind().layout().id.to_be_bytes()
            || dword(bytes, 13) != 14
            || dword(bytes, 17) != 1570
        {
            return None;
        }
        let r = &bytes[21..21 + RECORD];
        match dword(r, 0) {
            2 => {
                let l = self.input.kind().layout();
                Some(configuration_for(l.config, l.id))
            }
            1 => Some(control(r, &self.input.kind().layout(), &mut self.input)),
            7 => {
                if !self.input.accept(r) {
                    self.fail(2);
                    return self.remove();
                }
                None
            }
            _ => {
                self.fail(5);
                self.remove()
            }
        }
    }
    fn fail(&mut self, code: u32) {
        if self.error == 0 {
            self.error = code;
        }
    }
    pub(crate) fn error(&self) -> u32 {
        self.error
    }
    pub(crate) fn active(&mut self) -> bool {
        self.announced && !self.removed && self.error == 0 && self.input.ready()
    }
    pub(crate) fn external_input(&mut self, opcode: u8, a: u32, b: u32, c: u32) {
        if self.active() {
            self.input.command(opcode, a, b, c);
        }
    }
    pub(crate) fn release_input(&mut self) -> Option<[u8; FRAME]> {
        self.input.release()
    }
    pub(crate) fn remove(&mut self) -> Option<[u8; FRAME]> {
        if !self.announced || self.removed {
            return None;
        }
        if let Some(release) = self.release_input() {
            return Some(release);
        }
        self.removed = true;
        let mut r = [0; RECORD];
        put(&mut r, 0, 16);
        Some(frame(&r, self.input.kind().layout().id))
    }
    pub(crate) fn sent(&mut self, success: bool) {
        if !success {
            self.fail(4);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hid_mouse_enumeration_preserves_zero_ids_and_stalls_unknown_requests() {
        let mut m = Device::default();
        assert!(m.tick(0.0).is_none());
        let identity = m.tick(2000.0).unwrap();
        assert_eq!(&identity[8..12], &input::ID.to_be_bytes());
        assert_eq!(input::CONFIG[25], input::REPORT.len() as u8);
        let mut request = [0; RECORD];
        put(&mut request, 0, 1);
        request[4] = 0x81;
        request[5] = 6;
        request[6..8].copy_from_slice(&0x2200u16.to_le_bytes());
        request[1034..1036].copy_from_slice(&1024u16.to_le_bytes());
        let reply = m.incoming(&frame(&request, input::ID)).unwrap();
        assert_eq!(dword(&reply, 21 + 1036), 0);
        assert_eq!(dword(&reply, 21 + 1032), input::REPORT.len() as u32);
        assert_eq!(&reply[29..29 + input::REPORT.len()], input::REPORT);
        request[4] = 0x80;
        request[6..8].copy_from_slice(&0x03eeu16.to_le_bytes());
        assert_eq!(
            dword(&m.incoming(&frame(&request, input::ID)).unwrap(), 25) as i32,
            -9
        );
        assert_eq!(m.error(), 0);
        assert!(m.tick(12000.0).is_some());
        assert_eq!(m.error(), 1);
        assert!(!m.active());
    }
    #[test]
    fn hid_mouse_no_diagnostic_lifetime_limit_and_rejects_bad_reads() {
        let mut m = Device::default();
        m.tick(0.0);
        m.tick(2000.0);
        let mut r = [0; RECORD];
        put(&mut r, 0, 7);
        r[4] = 0x81;
        r[5] = 3;
        put(&mut r, 16, 7);
        assert!(m.incoming(&frame(&r, input::ID)).is_none());
        assert!(m.active());
        assert!(m.tick(100000000.0).is_none());
        assert!(m.active());
        r[4] = 1;
        assert!(m.incoming(&frame(&r, input::ID)).is_some());
        assert_eq!(m.error(), 2);
        assert!(!m.active());
    }
}
