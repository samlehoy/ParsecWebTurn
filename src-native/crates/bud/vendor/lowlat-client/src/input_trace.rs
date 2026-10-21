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
