//! Fixed, click-free absolute HID mapping experiment. Not live mouse routing.
use super::{FRAME, RECORD, dword, frame, input_probe, put};
use std::collections::VecDeque;

pub(super) const ID: u32 = 0xffff_f003;
pub(super) const DEVICE: &[u8] = &[
    18, 1, 0, 2, 0, 0, 0, 64, 0xff, 0xff, 3, 0xf0, 0, 1, 1, 2, 3, 1,
];
// Mouse / Pointer, five released buttons, absolute 16-bit X/Y in 0..32767.
// Non-boot protocol. No report ID, wheel, feature or output reports.
pub(super) const REPORT: &[u8] = &[
    0x05, 1, 0x09, 2, 0xa1, 1, 0x09, 1, 0xa1, 0, 0x05, 9, 0x19, 1, 0x29, 5, 0x15, 0, 0x25, 1, 0x95,
    5, 0x75, 1, 0x81, 2, 0x95, 1, 0x75, 3, 0x81, 3, 0x05, 1, 0x09, 0x30, 0x09, 0x31, 0x15, 0, 0x26,
    0xff, 0x7f, 0x75, 16, 0x95, 2, 0x81, 2, 0xc0, 0xc0,
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
    5,
    0,
    4,
];
// A 3x3 grid at 10%, 50%, 90%, in row order. No clicks at any point.
const GRID: [u16; 3] = [3277, 16384, 29490];
#[derive(Debug, Default)]
pub(super) struct Input {
    pending: VecDeque<input_probe::Read>,
    started: Option<u32>,
    step: usize,
}
impl Input {
    pub(super) fn accept(&mut self, r: &[u8], now: u32) -> bool {
        let Some(read) = input_probe::read(r) else {
            return false;
        };
        if dword(r, 16) < 5 || self.pending.len() >= 8 {
            return false;
        }
        self.started.get_or_insert(now);
        self.pending.push_back(read);
        true
    }
    pub(super) fn next(&mut self, now: u32) -> Option<([u8; FRAME], usize)> {
        if self.step >= 9 || now.saturating_sub(self.started?) < 1500 + self.step as u32 * 1000 {
            return None;
        }
        let request = self.pending.pop_front()?;
        let step = self.step;
        self.step += 1;
        let mut r = [0; RECORD];
        put(&mut r, 0, 8);
        r[4] = 0x81;
        r[5] = 3;
        put(&mut r, 8, request.id);
        put(&mut r, 16, 5);
        // r[20] remains zero: no press, release or synthetic button action.
        r[21..23].copy_from_slice(&GRID[step % 3].to_le_bytes());
        r[23..25].copy_from_slice(&GRID[step / 3].to_le_bytes());
        Some((frame(&r), step))
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
    #[test]
    fn usb_mouse_probe_absolute_grid_is_click_free_and_preserves_read_ids() {
        let mut input = Input::default();
        assert!(input.next(9999).is_none());
        assert!(!input.accept(&request(1, 3), 100));
        for step in 0..9 {
            let now = 1600 + step * 1000;
            assert!(input.accept(&request(step + 1, 5), 100));
            assert!(input.next(now - 1).is_none());
            let (wire, index) = input.next(now).unwrap();
            assert_eq!(index, step as usize);
            assert_eq!(dword(&wire, 29), step + 1);
            assert_eq!(dword(&wire, 37), 5);
            assert_eq!(wire[41], 0);
            assert_eq!(
                u16::from_le_bytes(wire[42..44].try_into().unwrap()),
                GRID[index % 3]
            );
            assert_eq!(
                u16::from_le_bytes(wire[44..46].try_into().unwrap()),
                GRID[index / 3]
            );
        }
        assert!(input.next(30000).is_none());
    }
    #[test]
    fn usb_mouse_probe_absolute_pending_reads_are_bounded() {
        let mut input = Input::default();
        for id in 0..8 {
            assert!(input.accept(&request(id, 5), 0));
        }
        assert!(!input.accept(&request(9, 5), 0));
    }
}
