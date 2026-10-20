//! Wire-contract tests for the opt-in input experiment; never send network data.
use super::input_probe::{completion, read};
use super::{RECORD, dword, put};

const ENDPOINT: u8 = 0x81;
const INTERRUPT: u8 = 3;
const CANCELLED: u32 = 0xc001_0000;

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
            let wire = completion(&incoming, Some([0; 3]));
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
    let wire = completion(&read(&request(42, 64)).unwrap(), None);
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
