//! Offline contract fixture only. Not linked into either executable and never
//! called by a live session. Layout evidence is recorded in the USB research doc.
use super::{FRAME, RECORD, dword, frame, put};

const ENDPOINT: u8 = 0x81;
const INTERRUPT: u8 = 3;
const REPORT_BYTES: usize = 3;
const CANCELLED: u32 = 0xc001_0000;

#[derive(Debug, PartialEq, Eq)]
struct Read {
    id: u32,
    capacity: u32,
}

fn read(record: &[u8]) -> Option<Read> {
    if record.len() != RECORD
        || dword(record, 0) != 7
        || record[4] != ENDPOINT
        || record[5] != INTERRUPT
    {
        return None;
    }
    let capacity = dword(record, 16);
    if !(3..=1550).contains(&capacity) {
        return None;
    }
    Some(Read {
        id: dword(record, 8),
        capacity,
    })
}

fn completion(request: &Read, cancelled: bool) -> [u8; FRAME] {
    let mut record = [0; RECORD];
    put(&mut record, 0, 8);
    record[4] = ENDPOINT;
    record[5] = INTERRUPT;
    put(&mut record, 8, request.id);
    if cancelled {
        put(&mut record, 12, CANCELLED);
    } else {
        put(&mut record, 16, REPORT_BYTES as u32);
        // Three zero bytes = released buttons, zero relative X and Y.
        // The extra Windows HID report-ID byte is NOT on the USB wire.
        record[20..23].copy_from_slice(&[0, 0, 0]);
    }
    frame(&record)
}

fn request(id: u32, capacity: u32) -> [u8; RECORD] {
    let mut record = [0; RECORD];
    put(&mut record, 0, 7);
    record[4] = ENDPOINT;
    record[5] = INTERRUPT;
    put(&mut record, 8, id);
    put(&mut record, 16, capacity);
    record
}

#[test]
fn usb_mouse_probe_neutral_interrupt_preserves_id_and_usb_report_size() {
    for id in [0, 1, 0xabcdef12] {
        for capacity in [3, 64, 1550] {
            let incoming = read(&request(id, capacity)).unwrap();
            assert_eq!(incoming, Read { id, capacity });
            let wire = completion(&incoming, false);
            assert_eq!(wire.len(), 1945);
            assert_eq!(wire[12], 32);
            assert_eq!(dword(&wire, 21), 8);
            assert_eq!(&wire[25..27], &[0x81, 3]);
            assert_eq!(dword(&wire, 29), id);
            assert_eq!(dword(&wire, 33), 0);
            assert_eq!(dword(&wire, 37), 3);
            assert_eq!(&wire[41..44], &[0, 0, 0]);
            assert!(wire[44..].iter().all(|byte| *byte == 0));
        }
    }
}

#[test]
fn usb_mouse_probe_cancellation_has_no_input_payload() {
    let wire = completion(&read(&request(42, 64)).unwrap(), true);
    assert_eq!(dword(&wire, 29), 42);
    assert_eq!(dword(&wire, 33), CANCELLED);
    assert_eq!(dword(&wire, 37), 0);
    assert!(wire[41..].iter().all(|byte| *byte == 0));
}

#[test]
fn usb_mouse_probe_interrupt_rejects_wrong_direction_type_and_bounds() {
    let good = request(7, 64);
    for len in 0..RECORD {
        assert!(read(&good[..len]).is_none());
    }
    assert!(read(&vec![0; RECORD + 1]).is_none());
    for capacity in [0, 1, 2, 1551, u32::MAX] {
        assert!(read(&request(7, capacity)).is_none());
    }
    for (offset, value) in [(0, 5), (4, 1), (4, 0x82), (5, 2)] {
        let mut invalid = good;
        invalid[offset] = value;
        assert!(read(&invalid).is_none());
    }
}
