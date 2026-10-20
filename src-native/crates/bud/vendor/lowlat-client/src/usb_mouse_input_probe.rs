//! Fixed desktop-only input experiment, gated by the USB probe feature and a
//! separate runtime opt-in. No physical input capture or arbitrary reports.
use super::{FRAME, RECORD, dword, frame, put};
use std::collections::VecDeque;

const LIMIT: usize = 8;
const CANCELLED: u32 = 0xc001_0000;
// Times relative to the first valid interrupt read. Two quiet seconds precede
// movement, and another quiet interval follows the released-button report.
const STEPS: &[(u32, [u8; 3])] = &[
    (0, [0, 0, 0]),
    (2000, [0, 12, 0]),
    (2250, [0, 12, 0]),
    (2500, [0, 12, 0]),
    (3000, [0, 244, 0]),
    (3250, [0, 244, 0]),
    (3500, [0, 244, 0]),
    (5000, [1, 0, 0]),
    (5120, [0, 0, 0]),
    (7000, [0, 0, 0]),
];

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Read {
    id: u32,
}

pub(super) fn read(record: &[u8]) -> Option<Read> {
    if record.len() != RECORD
        || dword(record, 0) != 7
        || record[4] != 0x81
        || record[5] != 3
        || !(3..=1550).contains(&dword(record, 16))
    {
        return None;
    }
    Some(Read {
        id: dword(record, 8),
    })
}

pub(super) fn completion(request: &Read, report: Option<[u8; 3]>) -> [u8; FRAME] {
    let mut r = [0; RECORD];
    put(&mut r, 0, 8);
    r[4] = 0x81;
    r[5] = 3;
    put(&mut r, 8, request.id);
    if let Some(report) = report {
        put(&mut r, 16, 3);
        r[20..23].copy_from_slice(&report);
    } else {
        put(&mut r, 12, CANCELLED);
    }
    frame(&r)
}

#[derive(Debug, Default)]
pub(super) struct Input {
    pending: VecDeque<Read>,
    started: Option<u32>,
    last_sent: Option<u32>,
    step: usize,
    pressed: bool,
}

impl Input {
    pub(super) fn accept(&mut self, r: &[u8], now: u32) -> bool {
        let Some(request) = read(r) else { return false };
        if self.pending.len() == LIMIT {
            return false;
        }
        self.started.get_or_insert(now);
        self.pending.push_back(request);
        true
    }

    pub(super) fn next(&mut self, now: u32) -> Option<([u8; FRAME], usize)> {
        let start = self.started?;
        let &(due, report) = STEPS.get(self.step)?;
        if now.saturating_sub(start) < due
            || self
                .last_sent
                .is_some_and(|last| now.saturating_sub(last) < 120)
            || self.pending.is_empty()
        {
            return None;
        }
        // Reserve a second outstanding read for release before pressing.
        // If unavailable, omit the click entirely rather than risk a held key.
        if report[0] != 0 && self.pending.len() < 2 {
            self.step += 1;
            return None;
        }
        let request = self.pending.pop_front()?;
        let step = self.step;
        self.step += 1;
        self.last_sent = Some(now);
        self.pressed = report[0] != 0;
        Some((completion(&request, Some(report)), step))
    }

    pub(super) fn release(&mut self) -> Option<[u8; FRAME]> {
        if !self.pressed {
            return None;
        }
        let request = self.pending.pop_front()?;
        self.pressed = false;
        Some(completion(&request, Some([0; 3])))
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
        put(&mut r, 16, 3);
        r
    }

    #[test]
    fn fixed_sequence_requires_reads_and_reserves_release() {
        let mut input = Input::default();
        assert!(input.next(9000).is_none());
        let mut reports = Vec::new();
        for &(due, expected) in STEPS {
            while input.pending.len() < 2 {
                assert!(input.accept(&request(42), 1000));
            }
            let (wire, step) = input.next(1000 + due).unwrap();
            assert_eq!(STEPS[step].1, expected);
            assert_eq!(&wire[41..44], &expected);
            assert_eq!(dword(&wire, 29), 42);
            assert_eq!(dword(&wire, 37), 3);
            reports.push(expected);
        }
        assert_eq!(reports.iter().filter(|r| r[0] != 0).count(), 1);
        assert_eq!(reports.last(), Some(&[0; 3]));
        assert!(input.next(30_000).is_none());
        assert!(input.release().is_none());
    }

    #[test]
    fn late_ticks_never_collapse_press_and_release_and_exit_releases() {
        let mut input = Input {
            step: 7,
            ..Input::default()
        };
        assert!(input.accept(&request(0), 0));
        assert!(input.accept(&request(99), 0));
        assert!(input.next(9000).is_some());
        assert!(input.next(9001).is_none());
        let release = input.release().unwrap();
        assert_eq!(dword(&release, 29), 99);
        assert_eq!(&release[41..44], &[0; 3]);
        assert!(input.release().is_none());
        let mut single = Input {
            step: 7,
            ..Input::default()
        };
        assert!(single.accept(&request(1), 0));
        assert!(single.next(5000).is_none());
        assert!(!single.pressed);
        assert_eq!(single.step, 8);
    }

    #[test]
    fn pending_queue_and_peer_fields_are_bounded() {
        let mut input = Input::default();
        for id in 0..LIMIT {
            assert!(input.accept(&request(id as u32), 0));
        }
        assert!(!input.accept(&request(90), 0));
        for (offset, value) in [(0, 5), (4, 1), (4, 0x82), (5, 2)] {
            let mut r = request(1);
            r[offset] = value;
            assert!(read(&r).is_none());
        }
        for size in [0, 1, 2, 1551, u32::MAX] {
            let mut r = request(1);
            put(&mut r, 16, size);
            assert!(read(&r).is_none());
        }
        for len in 0..RECORD {
            assert!(read(&request(1)[..len]).is_none());
        }
    }
}
