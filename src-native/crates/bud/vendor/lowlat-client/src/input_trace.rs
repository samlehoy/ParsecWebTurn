//! Temporary test-branch mouse tracing; absent from normal builds.
use std::sync::OnceLock;

type Sink = fn(&'static str, [i32; 4]);
static SINK: OnceLock<Sink> = OnceLock::new();

pub fn install(sink: Sink) {
    let _ = SINK.set(sink);
}
pub fn record(stage: &'static str, values: [i32; 4]) {
    if let Some(sink) = SINK.get() {
        sink(stage, values);
    }
}
pub fn button_command(stage: &'static str, frame: &[u8]) {
    if frame.len() == lowlat_core::control::CONTROL_HEADER_LEN
        && frame[12] == lowlat_core::control::op::MOUSE_BUTTON
    {
        let word =
            |offset| i32::from_be_bytes(frame[offset..offset + 4].try_into().unwrap_or_default());
        record(stage, [1, word(0), word(4), word(8)]);
    }
}
/// Aggregate control-channel pressure and smoothed RTT, not a message ACK.
pub fn pressure(stage: &'static str, session: &lowlat_core::session::Session<'_>) {
    let Some(p) = session.send_pressure(lowlat_core::control::CONTROL_CHANNEL) else {
        return;
    };
    let bounded = |value: u64| i32::try_from(value).unwrap_or(i32::MAX);
    let rtt = session.srtt_ms();
    record(
        stage,
        [
            bounded(u64::from(p.window)),
            bounded(p.nack_resends),
            bounded(p.timeout_resends),
            if rtt.is_finite() && rtt > 0.0 {
                (rtt * 1000.0).min(f64::from(i32::MAX)) as i32
            } else {
                -1
            },
        ],
    );
}
/// Only the numbered relative mouse collection contains button state.
pub fn mouse_buttons(frame: &[u8]) -> Option<u8> {
    if frame.get(12) != Some(&32)
        || frame.get(21..25) != Some(&[8, 0, 0, 0])
        || frame.get(37..41) != Some(&[8, 0, 0, 0])
    {
        return None;
    }
    let report = frame.get(41..49)?;
    (report[0] == 3).then_some(report[1])
}
/// Only the fixed cursor metadata from a complete, reassembled control message.
/// This is after transport delivery, not the socket's raw packet arrival time.
pub fn cursor_frame(stage: &'static str, frame: &[u8]) {
    if let Some(values) = cursor_values(frame) {
        record(stage, values);
    }
}
fn cursor_values(frame: &[u8]) -> Option<[i32; 4]> {
    if frame.len() < 34 || frame[12] != 9 {
        return None;
    }
    let word = |offset| i16::from_be_bytes([frame[offset], frame[offset + 1]]);
    let flags = u16::from_be_bytes([frame[32], frame[33]]);
    Some([
        i32::from(word(24)),
        i32::from(word(26)),
        i32::from(flags & 256 != 0),
        i32::from(flags & 512 != 0),
    ])
}
pub fn sent_frame(sent: bool, frame: &[u8]) {
    if frame.get(21..25) != Some(&[8, 0, 0, 0]) {
        return;
    }
    let Some(length) = frame
        .get(37..41)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map(u32::from_le_bytes)
    else {
        return;
    };
    if !matches!(length, 5 | 8) {
        return;
    }
    let Some(report) = frame.get(41..41 + length as usize) else {
        return;
    };
    if let Some((stage, values)) = decode_mouse_report(sent, report) {
        record(stage, values);
    }
}
fn decode_mouse_report(sent: bool, report: &[u8]) -> Option<(&'static str, [i32; 4])> {
    let word = |offset| {
        report
            .get(offset..offset + 2)
            .and_then(|bytes| <[u8; 2]>::try_from(bytes).ok())
    };
    match report.first() {
        Some(1) if report.len() == 5 => {
            if let (Some(x), Some(y)) = (word(1), word(3)) {
                return Some((
                    if sent {
                        "usb_sent_absolute"
                    } else {
                        "usb_failed_absolute"
                    },
                    [
                        i32::from(u16::from_le_bytes(x)),
                        i32::from(u16::from_le_bytes(y)),
                        0,
                        0,
                    ],
                ));
            }
        }
        Some(3) if report.len() == 8 => {
            if let (Some(buttons), Some(x), Some(y)) = (report.get(1), word(2), word(4)) {
                return Some((
                    if sent {
                        "usb_sent_relative"
                    } else {
                        "usb_failed_relative"
                    },
                    [
                        i32::from(i16::from_le_bytes(x)),
                        i32::from(i16::from_le_bytes(y)),
                        i32::from(*buttons),
                        0,
                    ],
                ));
            }
        }
        _ => {} // Never record keyboard reports or arbitrary USB payloads.
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn button_probe_accepts_only_a_complete_relative_mouse_report() {
        let mut frame = [0; 49];
        frame[12] = 32;
        frame[21..25].copy_from_slice(&8u32.to_le_bytes());
        frame[37..41].copy_from_slice(&8u32.to_le_bytes());
        frame[41] = 3;
        frame[42] = 5;
        assert_eq!(mouse_buttons(&frame), Some(5));
        for length in 0..49 {
            assert_eq!(mouse_buttons(&frame[..length]), None);
        }
        frame[41] = 2;
        assert_eq!(mouse_buttons(&frame), None); // Never sample keyboard reports.
        frame[41] = 3;
        frame[12] = 17;
        assert_eq!(mouse_buttons(&frame), None); // Never sample user data.
    }
    #[test]
    fn cursor_probe_excludes_other_messages_and_truncated_headers() {
        let mut frame = [0; 34];
        frame[12] = 9;
        frame[24..26].copy_from_slice(&(-12i16).to_be_bytes());
        frame[26..28].copy_from_slice(&320i16.to_be_bytes());
        frame[32..34].copy_from_slice(&0x200u16.to_be_bytes());
        assert_eq!(cursor_values(&frame), Some([-12, 320, 0, 1]));
        for length in 0..34 {
            assert!(cursor_values(&frame[..length]).is_none());
        }
        frame[12] = 8;
        assert!(cursor_values(&frame).is_none());
    }
    #[test]
    fn trace_decodes_only_complete_mouse_reports() {
        assert_eq!(
            decode_mouse_report(true, &[1, 0, 128, 255, 127]),
            Some(("usb_sent_absolute", [32768, 32767, 0, 0]))
        );
        assert_eq!(
            decode_mouse_report(true, &[3, 1, 255, 255, 2, 0, 0, 0]),
            Some(("usb_sent_relative", [-1, 2, 1, 0]))
        );
        assert!(decode_mouse_report(true, &[2; 30]).is_none());
        assert!(decode_mouse_report(true, &[1; 30]).is_none());
        assert!(decode_mouse_report(true, &[3; 5]).is_none());
    }
}
