//! Absolute desktop mouse reports, bounded independently of session lifetime.
use super::{FRAME, RECORD, dword, frame, put};
use std::collections::VecDeque;
pub(super) const ID: u32 = 0xffff_f004;
pub(super) const DEVICE: &[u8] = &[
    18, 1, 0, 2, 0, 0, 0, 64, 0xff, 0xff, 4, 0xf0, 0, 1, 1, 2, 3, 1,
];
// Same buttons and wheels as relative mode, with absolute 16-bit X/Y.
pub(super) const REPORT: &[u8] = &[
    0x05, 1, 0x09, 2, 0xa1, 1, 0x09, 1, 0xa1, 0, 0x05, 9, 0x19, 1, 0x29, 5, 0x15, 0, 0x25, 1, 0x95,
    5, 0x75, 1, 0x81, 2, 0x95, 1, 0x75, 3, 0x81, 3, 0x05, 1, 0x09, 0x30, 0x09, 0x31, 0x15, 0, 0x26,
    0xff, 0x7f, 0x75, 16, 0x95, 2, 0x81, 2, 0x15, 0x81, 0x25, 0x7f, 0x75, 8, 0x95, 1, 0x09, 0x38,
    0x81, 6, 0x05, 0x0c, 0x0a, 0x38, 0x02, 0x81, 6, 0xc0, 0xc0,
];
pub(super) const CONFIG: &[u8] = &[
    9,
    2,
    34,
    0,
    1,
    1,
    0,
    0x80,
    50,
    9,
    4,
    0,
    0,
    1,
    3,
    0,
    0,
    0,
    9,
    0x21,
    0x11,
    1,
    0,
    1,
    0x22,
    REPORT.len() as u8,
    0,
    7,
    5,
    0x81,
    3,
    7,
    0,
    4,
];

#[derive(Debug, Clone, Copy)]
struct Read {
    id: u32,
}
fn read(record: &[u8]) -> Option<Read> {
    if record.len() != RECORD
        || dword(record, 0) != 7
        || record[4] != 0x81
        || record[5] != 3
        || !(7..=1550).contains(&dword(record, 16))
    {
        return None;
    }
    Some(Read {
        id: dword(record, 8),
    })
}
#[derive(Debug, Default, Clone, Copy)]
struct Sample {
    buttons: u8,
    edge: bool,
    point: Option<(u16, u16)>,
    vertical: i64,
    horizontal: i64,
}
#[derive(Debug, Default)]
pub(super) struct Input {
    pending: VecDeque<Read>,
    samples: VecDeque<Sample>,
    position: Option<(u16, u16)>,
    sent_position: Option<(u16, u16)>,
    buttons: u8,
    sent_buttons: u8,
    wheel_remainder: (i64, i64),
    ready: bool,
    failed: bool,
    last_sent: Option<u64>,
}
impl Input {
    pub(super) fn ready(&self) -> bool {
        self.ready && !self.failed
    }
    pub(super) fn failed(&self) -> bool {
        self.failed
    }
    pub(super) fn accept(&mut self, record: &[u8]) -> bool {
        let Some(request) = read(record) else {
            return false;
        };
        if self.pending.len() >= 8 {
            return false;
        }
        self.pending.push_back(request);
        self.ready = true;
        true
    }
    fn enqueue(&mut self, sample: Sample) {
        if let Some(last) = self.samples.back_mut().filter(|last| {
            last.buttons == sample.buttons
                && !last.edge
                && !sample.edge
                && ((last.vertical == 0 && last.horizontal == 0) || last.point == sample.point)
        }) {
            if sample.point.is_some() {
                last.point = sample.point;
            }
            last.vertical += sample.vertical;
            last.horizontal += sample.horizontal;
            if last.vertical.abs() > 8192 || last.horizontal.abs() > 8192 {
                self.failed = true;
            }
        } else if self.samples.len() < 64 {
            self.samples.push_back(sample);
        } else {
            self.failed = true;
        }
    }
    pub(super) fn command(&mut self, opcode: u8, a: u32, b: u32, c: u32) {
        if opcode == 24 {
            self.reset();
            return;
        }
        if !self.ready || self.failed {
            return;
        }
        if opcode == 3 {
            // Mode 2 is private to the app/USB adapter, never sent as Parsec input.
            if a != 2 || b > 32767 || c > 32767 {
                return;
            }
            let point = (b as u16, c as u16);
            if self.position == Some(point) {
                return;
            }
            self.position = Some(point);
        }
        let Some(point) = self.position else {
            return;
        };
        let mut sample = Sample {
            buttons: self.buttons,
            point: Some(point),
            ..Default::default()
        };
        match opcode {
            1 => {
                let bit = match a {
                    1 => 1,
                    2 => 4,
                    3 => 2,
                    4 => 8,
                    5 => 16,
                    _ => return,
                };
                if b > 1 {
                    return;
                }
                if b != 0 {
                    self.buttons |= bit;
                } else {
                    self.buttons &= !bit;
                }
                sample.buttons = self.buttons;
                sample.edge = true;
            }
            2 => {
                self.wheel_remainder.0 += i64::from(a as i32);
                self.wheel_remainder.1 += i64::from(b as i32);
                sample.horizontal = self.wheel_remainder.0 / 120;
                sample.vertical = self.wheel_remainder.1 / 120;
                self.wheel_remainder.0 %= 120;
                self.wheel_remainder.1 %= 120;
                if sample.horizontal == 0 && sample.vertical == 0 {
                    return;
                }
            }
            3 => {}
            _ => return,
        }
        if sample.horizontal.abs() > 8192 || sample.vertical.abs() > 8192 {
            self.failed = true;
            return;
        }
        self.enqueue(sample);
    }
    fn packet(
        request: Read,
        buttons: u8,
        point: (u16, u16),
        vertical: u8,
        horizontal: u8,
    ) -> [u8; FRAME] {
        let mut record = [0; RECORD];
        put(&mut record, 0, 8);
        record[4] = 0x81;
        record[5] = 3;
        put(&mut record, 8, request.id);
        put(&mut record, 16, 7);
        record[20] = buttons;
        record[21..23].copy_from_slice(&point.0.to_le_bytes());
        record[23..25].copy_from_slice(&point.1.to_le_bytes());
        record[25] = vertical;
        record[26] = horizontal;
        frame(&record)
    }
    pub(super) fn next(&mut self, now: u64) -> Option<[u8; FRAME]> {
        if self.failed
            || self.pending.is_empty()
            || self.last_sent.is_some_and(|t| now.saturating_sub(t) < 4)
        {
            return None;
        }
        let sample = self.samples.front_mut()?;
        let point = sample.point.take().or(self.sent_position)?;
        let take = |value: &mut i64| {
            let n = (*value).clamp(-127, 127);
            *value -= n;
            n as i8 as u8
        };
        let packet = Self::packet(
            self.pending.pop_front()?,
            sample.buttons,
            point,
            take(&mut sample.vertical),
            take(&mut sample.horizontal),
        );
        self.sent_position = Some(point);
        self.sent_buttons = sample.buttons;
        self.last_sent = Some(now);
        if sample.vertical == 0 && sample.horizontal == 0 {
            self.samples.pop_front();
        }
        Some(packet)
    }
    pub(super) fn reset(&mut self) {
        self.buttons = 0;
        self.position = None;
        self.wheel_remainder = (0, 0);
        self.samples.clear();
        if self.ready && self.sent_position.is_some() {
            self.samples.push_back(Sample {
                point: self.sent_position,
                ..Default::default()
            });
        }
    }
    pub(super) fn release(&mut self) -> Option<[u8; FRAME]> {
        self.reset();
        if self.sent_buttons == 0 {
            return None;
        }
        let request = self.pending.pop_front()?;
        let point = self.sent_position?;
        self.sent_buttons = 0;
        Some(Self::packet(request, 0, point, 0, 0))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request(id: u32, capacity: u32) -> [u8; RECORD] {
        let mut r = [0; RECORD];
        put(&mut r, 0, 7);
        r[4] = 0x81;
        r[5] = 3;
        put(&mut r, 8, id);
        put(&mut r, 16, capacity);
        r
    }
    fn ready() -> Input {
        let mut i = Input::default();
        assert!(i.accept(&request(0, 7)));
        assert!(i.next(0).is_none());
        i
    }
    fn report(i: &mut Input, t: u64) -> [u8; 7] {
        assert!(i.accept(&request(t as u32, 7)));
        let f = i.next(t).unwrap();
        assert_eq!(dword(&f, 37), 7);
        f[41..48].try_into().unwrap()
    }
    fn xy(r: [u8; 7]) -> (u16, u16) {
        (
            u16::from_le_bytes([r[1], r[2]]),
            u16::from_le_bytes([r[3], r[4]]),
        )
    }
    #[test]
    fn hid_mouse_click_drag_release_keep_edge_positions() {
        let mut i = ready();
        i.command(1, 1, 1, 0);
        assert_eq!(i.buttons, 0);
        i.command(3, 2, 100, 200);
        i.command(1, 1, 1, 0);
        i.command(3, 2, 30000, 32767);
        i.command(1, 1, 0, 0);
        assert_eq!(xy(report(&mut i, 4)), (100, 200));
        let press = report(&mut i, 8);
        assert_eq!((press[0], xy(press)), (1, (100, 200)));
        let drag = report(&mut i, 12);
        assert_eq!((drag[0], xy(drag)), (1, (30000, 32767)));
        let release = report(&mut i, 16);
        assert_eq!((release[0], xy(release)), (0, (30000, 32767)));
    }
    #[test]
    fn hid_mouse_wheel_target_and_focus_release_discard_stale_motion() {
        let mut i = ready();
        i.command(3, 2, 500, 600);
        let _ = report(&mut i, 4);
        i.command(2, 60, (-60i32) as u32, 0);
        assert!(i.samples.is_empty());
        i.command(2, 60, (-60i32) as u32, 0);
        i.command(3, 2, 700, 800);
        let wheel = report(&mut i, 8);
        assert_eq!(xy(wheel), (500, 600));
        assert_eq!(&wheel[5..], [255, 1]);
        let _ = report(&mut i, 12);
        i.command(1, 3, 1, 0);
        assert_eq!(report(&mut i, 16)[0], 2);
        i.command(3, 2, 900, 1000);
        let release = i.release().unwrap();
        assert_eq!(release[41], 0);
        assert_eq!(xy(release[41..48].try_into().unwrap()), (700, 800));
        i.command(1, 1, 1, 0);
        assert_eq!(i.buttons, 0);
    }
    #[test]
    fn hid_mouse_bounds_and_long_lived_pacing() {
        let mut i = ready();
        assert!(!i.accept(&request(4, 5)));
        for n in 0..7 {
            assert!(i.accept(&request(n, 7)));
        }
        assert!(!i.accept(&request(9, 7)));
        i.command(3, 2, 1, 1);
        let now = u64::from(u32::MAX) + 100;
        assert!(i.next(now).is_some());
        i.command(3, 2, 2, 2);
        assert!(i.next(now + 1).is_none());
        assert!(i.next(now + 4).is_some());
        for n in 0..65 {
            i.command(1, 1, n % 2, 0);
        }
        assert!(i.failed());
        let mut i = ready();
        i.command(3, 1, 1, 1);
        assert!(i.samples.is_empty());
    }
}
