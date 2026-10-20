//! Test-build-only cursor policy. Host position controls display; hardware Raw
//! Input controls motion. No video composition, framebuffer readback or copying.
use std::{
    fs::File,
    io::Write,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
// Keep outside the main window's updater/settings/control message range.
pub const REFRESH: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 40;

pub struct Lifetime {
    window: Arc<crate::window::Window>,
    generation: u64,
    trace: Option<File>,
    started: Instant,
    next_sample: Instant,
}
impl Lifetime {
    pub fn new(window: Arc<crate::window::Window>, generation: u64) -> Self {
        let trace = if std::env::var("PARSEC_USB_MOUSE_LIVE_PROBE").as_deref() == Ok("1")
            && std::env::var("PARSEC_USB_MOUSE_PROBE").as_deref() == Ok("1")
        {
            std::env::var_os("PARSEC_USB_PROBE_LOG").and_then(|path| {
                let path = std::path::PathBuf::from(path).with_extension("cursor.jsonl");
                match File::options().write(true).create_new(true).open(path) {
                    Ok(file) => Some(file),
                    Err(_) => {
                        crate::event_log::record(
                            "WARN",
                            "hid_cursor_trace_unavailable",
                            serde_json::json!({}),
                        );
                        None
                    }
                }
            })
        } else {
            None
        };
        Self {
            window,
            generation,
            trace,
            started: Instant::now(),
            next_sample: Instant::now(),
        }
    }
    pub fn tick(&mut self) {
        if Instant::now() >= self.next_sample && self.started.elapsed() < Duration::from_secs(120) {
            self.write("hid_cursor_sample");
            self.next_sample = Instant::now() + Duration::from_secs(1);
        }
    }
    fn write(&mut self, event: &str) {
        let Some(file) = &mut self.trace else { return };
        let entry = serde_json::json!({"event":event,"unix_ms":unix_ms(),
            "elapsed_ms":self.started.elapsed().as_millis(),"generation":self.generation,
            "counters":self.window.hid_cursor_metrics.snapshot(),
            "display":self.window.hid_cursor_diagnostic()});
        if writeln!(file, "{entry}")
            .and_then(|_| file.flush())
            .is_err()
        {
            self.trace = None;
        }
    }
}
impl Drop for Lifetime {
    fn drop(&mut self) {
        self.write("hid_cursor_final");
        self.window.set_usb_cursor_active(self.generation, false);
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Default)]
pub struct Metrics {
    pub wire_events: AtomicU64,
    pub wire_position_changes: AtomicU64,
    pub wire_images: AtomicU64,
    pub delivered_events: AtomicU64,
    pub delivered_position_changes: AtomicU64,
    pub refresh_posted: AtomicU64,
    pub refresh_applied: AtomicU64,
    pub raw_motion_packets: AtomicU64,
    pub clip_successes: AtomicU64,
    pub clip_failures: AtomicU64,
    last_wire: Mutex<Option<(i16, i16)>>,
    wire_ms: AtomicU64,
    delivered_ms: AtomicU64,
}
impl Metrics {
    pub fn wire(&self, bytes: &[u8]) {
        if bytes.len() < 34 || bytes[12] != 9 {
            return;
        }
        let short = |i| i16::from_be_bytes([bytes[i], bytes[i + 1]]);
        let point = (short(24), short(26));
        let mut last = self.last_wire.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|old| old != point) {
            self.wire_position_changes.fetch_add(1, Ordering::Relaxed);
        }
        *last = Some(point);
        self.wire_events.fetch_add(1, Ordering::Relaxed);
        if bytes[16..20] != [0; 4] {
            self.wire_images.fetch_add(1, Ordering::Relaxed);
        }
        self.wire_ms.store(unix_ms(), Ordering::Relaxed);
    }
    pub fn delivered(&self, changed: bool) {
        self.delivered_events.fetch_add(1, Ordering::Relaxed);
        if changed {
            self.delivered_position_changes
                .fetch_add(1, Ordering::Relaxed);
        }
        self.delivered_ms.store(unix_ms(), Ordering::Relaxed);
    }
    fn snapshot(&self) -> serde_json::Value {
        let count = |v: &AtomicU64| v.load(Ordering::Relaxed);
        let age = |v: &AtomicU64| {
            let t = count(v);
            (t != 0).then(|| unix_ms().saturating_sub(t))
        };
        serde_json::json!({"wire_events":count(&self.wire_events),"wire_position_changes":count(&self.wire_position_changes),
            "wire_images":count(&self.wire_images),"wire_age_ms":age(&self.wire_ms),
            "delivered_events":count(&self.delivered_events),"delivered_position_changes":count(&self.delivered_position_changes),
            "delivered_age_ms":age(&self.delivered_ms),"refresh_posted":count(&self.refresh_posted),
            "refresh_applied":count(&self.refresh_applied),"raw_motion_packets":count(&self.raw_motion_packets),
            "clip_successes":count(&self.clip_successes),"clip_failures":count(&self.clip_failures)})
    }
}

/// The host metadata's position is meaningful for a visible desktop cursor.
/// Relative/hidden host modes retain the original hidden native capture.
pub fn visible_position(cursor: Option<(i32, i32, bool)>) -> Option<(i32, i32)> {
    cursor.and_then(|(x, y, relative)| (!relative).then_some((x, y)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hid_cursor_counts_wire_changes_without_logging_coordinates() {
        let metrics = Metrics::default();
        metrics.wire(&[0; 13]);
        let mut frame = [0; 34];
        frame[12] = 9;
        metrics.wire(&frame);
        metrics.wire(&frame);
        frame[24..26].copy_from_slice(&123i16.to_be_bytes());
        metrics.wire(&frame);
        metrics.delivered(false);
        let sample = metrics.snapshot();
        assert_eq!(sample["wire_events"], 3);
        assert_eq!(sample["wire_position_changes"], 1);
        assert_eq!(sample["delivered_events"], 1);
        assert_eq!(sample["delivered_position_changes"], 0);
        assert!(sample["wire_age_ms"].is_number());
        assert!(sample.get("positionX").is_none());
    }
    #[test]
    fn hid_cursor_never_invents_position_or_displays_a_hidden_host_cursor() {
        assert_eq!(visible_position(None), None);
        assert_eq!(visible_position(Some((100, 200, true))), None);
        assert_eq!(visible_position(Some((100, 200, false))), Some((100, 200)));
    }
}
