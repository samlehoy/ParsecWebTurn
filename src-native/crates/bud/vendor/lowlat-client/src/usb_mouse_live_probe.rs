//! Bounded desktop mouse forwarding for the isolated USB test build.
//! Consumes already-mapped application mouse commands, not global input.
use super::{FRAME, RECORD, dword, frame, input_probe, put};
use std::collections::VecDeque;

pub(super) const ID: u32 = 0xffff_f002;
pub(super) const DEVICE: &[u8] = &[
    18, 1, 0, 2, 0, 0, 0, 64, 0xff, 0xff, 2, 0xf0, 0, 1, 1, 2, 3, 1,
];
// Five buttons, signed relative X/Y, vertical wheel and Consumer AC Pan.
// A non-boot interface: the five-byte report is not a boot-mouse packet.
pub(super) const REPORT: &[u8] = &[
    0x05, 1, 0x09, 2, 0xa1, 1, 0x09, 1, 0xa1, 0, 0x05, 9, 0x19, 1, 0x29, 5, 0x15, 0, 0x25, 1, 0x95,
    5, 0x75, 1, 0x81, 2, 0x95, 1, 0x75, 3, 0x81, 3, 0x05, 1, 0x09, 0x30, 0x09, 0x31, 0x15, 0x81,
    0x25, 0x7f, 0x75, 8, 0x95, 2, 0x81, 6, 0x09, 0x38, 0x95, 1, 0x81, 6, 0x05, 0x0c, 0x0a, 0x38,
    0x02, 0x95, 1, 0x81, 6, 0xc0, 0xc0,
];
pub(super) const CONFIG: &[u8] = &[
    9, 2, 34, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 1, 3, 0, 0, 0, 9, 0x21, 0x11, 1, 0, 1, 0x22, 65, 0,
    7, 5, 0x81, 3, 5, 0, 4,
];
const QUEUE_LIMIT: usize = 64;
const MOTION_LIMIT: i64 = 8192;

#[derive(Debug, Default, Clone, Copy)]
struct Sample {
    buttons: u8,
    x: i64,
    y: i64,
    vertical: i64,
    horizontal: i64,
}
impl Sample {
    fn empty(self) -> bool {
        self.x == 0 && self.y == 0 && self.vertical == 0 && self.horizontal == 0
    }
}

#[derive(Debug, Default)]
pub(super) struct Input {
    pending: VecDeque<input_probe::Read>,
    samples: VecDeque<Sample>,
    buttons: u8,
    sent_buttons: u8,
    anchor: Option<(i32, i32)>,
    wheel_remainder: (i64, i64),
    ready: bool,
    faulted: bool,
    last_sent: Option<u32>,
    pub reports: u32,
    pub commands: u32,
    pub suppressed: u32,
}

impl Input {
    pub(super) fn buttons(&self) -> u8 {
        self.buttons
    }

    pub(super) fn accept(&mut self, r: &[u8]) -> bool {
        let Some(read) = input_probe::read(r) else {
            return false;
        };
        if dword(r, 16) < 5 || self.pending.len() == 8 {
            return false;
        }
        self.pending.push_back(read);
        if !self.ready {
            self.ready = true;
            self.samples.push_back(Sample::default());
        }
        true
    }

    fn enqueue(&mut self, sample: Sample) {
        if let Some(last) = self
            .samples
            .back_mut()
            .filter(|s| s.buttons == sample.buttons)
        {
            last.x = last.x.saturating_add(sample.x);
            last.y = last.y.saturating_add(sample.y);
            last.vertical = last.vertical.saturating_add(sample.vertical);
            last.horizontal = last.horizontal.saturating_add(sample.horizontal);
            if [last.x, last.y, last.vertical, last.horizontal]
                .iter()
                .any(|v| v.abs() > MOTION_LIMIT)
            {
                self.faulted = true;
            }
        } else if self.samples.len() < QUEUE_LIMIT {
            self.samples.push_back(sample);
        } else {
            self.faulted = true;
        }
    }

    pub(super) fn command(&mut self, opcode: u8, a: u32, b: u32, c: u32) {
        if opcode == 24 {
            self.reset();
            return;
        }
        self.suppressed = self.suppressed.saturating_add(1);
        if !self.ready || self.faulted {
            return;
        }
        self.commands = self.commands.saturating_add(1);
        let mut sample = Sample {
            buttons: self.buttons,
            ..Sample::default()
        };
        match opcode {
            1 => {
                // Parsec: left=1, middle=2, right=3, X1=4, X2=5.
                // HID: left=bit0, right=bit1, middle=bit2, X1/2=bit3/4.
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
            }
            2 => {
                self.wheel_remainder.0 += i64::from(a as i32);
                self.wheel_remainder.1 += i64::from(b as i32);
                sample.horizontal = self.wheel_remainder.0 / 120;
                sample.vertical = self.wheel_remainder.1 / 120;
                self.wheel_remainder.0 %= 120;
                self.wheel_remainder.1 %= 120;
                if sample.empty() {
                    return;
                }
            }
            3 if a == 1 => {
                self.anchor = None;
                sample.x = i64::from(b as i32);
                sample.y = i64::from(c as i32);
            }
            3 if a == 0 => {
                let point = (b as i32, c as i32);
                // Anchor first absolute sample; never snap the host cursor.
                let Some(previous) = self.anchor.replace(point) else {
                    return;
                };
                sample.x = i64::from(point.0) - i64::from(previous.0);
                sample.y = i64::from(point.1) - i64::from(previous.1);
            }
            _ => return, // Stream >0 motion is suppressed, not reinterpreted.
        }
        if [sample.x, sample.y, sample.vertical, sample.horizontal]
            .iter()
            .any(|v| v.abs() > MOTION_LIMIT)
        {
            self.faulted = true;
            return;
        }
        self.enqueue(sample);
    }

    pub(super) fn reset(&mut self) {
        self.buttons = 0;
        self.anchor = None;
        self.wheel_remainder = (0, 0);
        self.samples.clear();
        if self.ready {
            self.samples.push_back(Sample::default());
        }
    }

    fn packet(request: &input_probe::Read, report: [u8; 5]) -> [u8; FRAME] {
        let mut r = [0; RECORD];
        put(&mut r, 0, 8);
        r[4] = 0x81;
        r[5] = 3;
        put(&mut r, 8, request.id);
        put(&mut r, 16, 5);
        r[20..25].copy_from_slice(&report);
        frame(&r)
    }

    pub(super) fn next(&mut self, now: u32) -> Option<[u8; FRAME]> {
        if self.faulted
            || self.pending.is_empty()
            || self
                .last_sent
                .is_some_and(|last| now.saturating_sub(last) < 4)
        {
            return None;
        }
        let sample = self.samples.front_mut()?;
        let take = |value: &mut i64| {
            let part = (*value).clamp(-127, 127);
            *value -= part;
            part as i8 as u8
        };
        let report = [
            sample.buttons,
            take(&mut sample.x),
            take(&mut sample.y),
            take(&mut sample.vertical),
            take(&mut sample.horizontal),
        ];
        self.sent_buttons = sample.buttons;
        if sample.empty() {
            self.samples.pop_front();
        }
        self.last_sent = Some(now);
        self.reports = self.reports.saturating_add(1);
        Some(Self::packet(&self.pending.pop_front()?, report))
    }

    pub(super) fn release(&mut self) -> Option<[u8; FRAME]> {
        self.reset();
        if self.sent_buttons == 0 {
            return None;
        }
        let request = self.pending.pop_front()?;
        self.sent_buttons = 0;
        Some(Self::packet(&request, [0; 5]))
    }

    pub(super) fn failed(&self) -> bool {
        self.faulted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(id: u32) -> [u8; RECORD] {
        let mut r = [0; RECORD];
        put(&mut r, 0, 7);
        r[4] = 0x81;
        r[5] = 3;
        put(&mut r, 8, id);
        put(&mut r, 16, 5);
        r
    }
    fn ready() -> Input {
        let mut input = Input::default();
        assert!(input.accept(&request(1)));
        assert!(input.next(0).is_some());
        input
    }
    fn report(input: &mut Input, now: u32) -> [u8; 5] {
        assert!(input.accept(&request(now)));
        let p = input.next(now).unwrap();
        assert_eq!(dword(&p, 29), now);
        assert_eq!(dword(&p, 37), 5);
        p[41..46].try_into().unwrap()
    }
    #[test]
    fn usb_mouse_probe_live_preserves_drag_edges_and_splits_motion() {
        let mut input = ready();
        input.command(1, 1, 1, 0);
        input.command(3, 1, 300, (-260i32) as u32);
        input.command(1, 1, 0, 0);
        assert_eq!(report(&mut input, 4), [1, 127, 129, 0, 0]);
        assert_eq!(report(&mut input, 8), [1, 127, 129, 0, 0]);
        assert_eq!(report(&mut input, 12), [1, 46, 250, 0, 0]);
        assert_eq!(report(&mut input, 16), [0; 5]);
        assert!(input.next(100).is_none());
    }
    #[test]
    fn usb_mouse_probe_live_buttons_and_fractional_wheels() {
        let mut input = ready();
        for (n, bit) in [(1, 1), (2, 4), (3, 2), (4, 8), (5, 16)] {
            input.command(1, n, 1, 0);
            assert_eq!(report(&mut input, n * 4)[0], bit);
            input.reset();
            let _ = report(&mut input, n * 4 + 100);
            input.last_sent = None;
        }
        input.command(2, 60, (-60i32) as u32, 0);
        assert!(input.samples.is_empty());
        input.command(2, 60, (-60i32) as u32, 0);
        assert_eq!(report(&mut input, 1000), [0, 0, 0, 255, 1]);
    }
    #[test]
    fn usb_mouse_probe_live_focus_release_discards_stale_motion_and_anchor() {
        let mut input = ready();
        input.command(3, 0, 100, 100);
        assert!(input.samples.is_empty());
        input.command(3, 0, 110, 95);
        assert_eq!(report(&mut input, 4), [0, 10, 251, 0, 0]);
        input.command(1, 3, 1, 0);
        assert_eq!(report(&mut input, 8)[0], 2);
        input.command(3, 1, 1000, 0);
        input.command(24, 0, 0, 0);
        assert_eq!(report(&mut input, 12), [0; 5]);
        input.command(3, 0, 500, 500);
        assert!(input.samples.is_empty());
    }
    #[test]
    fn usb_mouse_probe_live_capacity_queue_and_backlog_are_bounded() {
        let mut input = ready();
        let mut small = request(4);
        put(&mut small, 16, 3);
        assert!(!input.accept(&small));
        for n in 0..=QUEUE_LIMIT {
            input.command(1, 1, (n % 2) as u32, 0);
        }
        assert!(input.failed());
        let mut input = ready();
        input.command(3, 1, 8192, 0);
        input.command(3, 1, 1, 0);
        assert!(input.failed());
    }
}
