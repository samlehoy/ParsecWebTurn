//! Shared session commands and presentation state, independent of window APIs.
//! Native views own their controls and notifications; media workers stay separate.
use super::{host_displays::Displays, profile::Video};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

#[derive(Clone, Debug)]
pub enum Event {
    NativeSession(Command),
    Size(i32, i32),
    Focus(bool),
    Clipboard,
    // Local scheduling hint only; never translated into remote mouse input.
    CursorReady,
    Motion(i32, i32),
    RelativeMotion(i32, i32),
    RelativeMode(bool),
    Button(bool, i32, i32, i32),
    Text(u32),
    Key(bool, &'static str, i32),
    Scroll(i32, i32),
    Fullscreen(bool),
    #[cfg(feature = "bud")]
    HidInputRelease,
}

#[derive(Clone, Debug)]
pub enum Command {
    Apply(Video),
    SelectDisplay(String),
    RefreshDisplays,
    Disconnect,
    HideButton,
    SecureAttention,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Applied,
    Failed,
    Confirmed,
    Waiting,
    Unconfirmed,
    AttentionSent,
    AttentionFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notification {
    Status(Status),
    DisplaysChanged,
    DisplaySelected,
}

/// Only the services needed by the host-settings controller. A view translates
/// notifications to its own event mechanism; it never drives the video worker.
pub trait View {
    fn session(&self) -> &State;
    fn display_size(&self) -> (u32, u32);
    fn notify(&self, notification: Notification);

    fn waiting(&self) {
        self.session().video_pending.store(true, Ordering::Release);
        self.notify(Notification::Status(Status::Waiting));
    }
    fn finished(&self, confirmed: bool) {
        self.session().video_pending.store(false, Ordering::Release);
        self.notify(Notification::Status(if confirmed {
            Status::Confirmed
        } else {
            Status::Unconfirmed
        }));
    }
    fn result(&self, ok: bool) {
        self.notify(Notification::Status(if ok {
            Status::Applied
        } else {
            Status::Failed
        }));
    }
}

/// Host preferences, not the actual decoded stream or capture confirmation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostVideo {
    pub bandwidth_mbps: Option<u32>,
    pub fps: Option<u32>,
    pub constant_fps: Option<bool>,
    pub rotated: Option<bool>,
}
impl HostVideo {
    pub fn from_report(report: &Value) -> Self {
        let number = |key: &str, max| {
            report[key]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| (1..=max).contains(n))
        };
        Self {
            bandwidth_mbps: number("bandwidth_mbps", 100),
            fps: number("fps", 240),
            constant_fps: report["constant_fps"].as_bool(),
            rotated: report["rotated"].as_bool(),
        }
    }

    fn panel_video(self, mut saved: Video) -> Video {
        saved.bandwidth_mbps = Some(saved.bandwidth_mbps.or(self.bandwidth_mbps).unwrap_or(10));
        saved.fps = Some(saved.fps.or(self.fps).unwrap_or(60));
        saved.constant_fps = Some(saved.constant_fps.or(self.constant_fps).unwrap_or(false));
        saved.rotated = Some(saved.rotated.or(self.rotated).unwrap_or(false));
        saved
    }
}

/// Keeps the existing lock/atomic boundaries; no new queue, worker or polling.
#[derive(Default)]
pub struct State {
    pub host_video: Mutex<HostVideo>,
    pub displays: Mutex<Displays>,
    pub video_pending: AtomicBool,
    pub hid_primary_ready: AtomicBool,
}
impl State {
    pub fn panel_video(&self, saved: Video) -> Video {
        self.host_video
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .panel_video(saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_core::profile::Resolution;

    #[test]
    fn reopening_panel_preserves_saved_requests_and_local_controls() {
        let state = State::default();
        *state.host_video.lock().unwrap() = HostVideo::from_report(&serde_json::json!({
            "bandwidth_mbps":80,"fps":144,"constant_fps":false,"rotated":true
        }));
        let saved = Video {
            resolution: Resolution::Fixed(1920, 1080),
            bandwidth_mbps: Some(50),
            fps: Some(60),
            constant_fps: Some(true),
            rotated: Some(false),
            immersive: 3,
            fullscreen: true,
        };
        assert_eq!(state.panel_video(saved.clone()), saved);
        let host = state.panel_video(Video::default());
        assert_eq!(host.bandwidth_mbps, Some(80));
        assert_eq!(host.fps, Some(144));
        assert_eq!(host.constant_fps, Some(false));
        assert_eq!(host.rotated, Some(true));
        assert_eq!(host.resolution, Resolution::Host);
        assert!(!host.fullscreen);
    }

    #[test]
    fn unknown_or_out_of_range_host_preferences_keep_existing_panel_defaults() {
        for report in [
            Value::Null,
            serde_json::json!({"bandwidth_mbps":0,"fps":241,"constant_fps":1,"rotated":"true"}),
            serde_json::json!({"bandwidth_mbps":101,"fps":-1}),
            serde_json::json!({"bandwidth_mbps":u64::MAX,"fps":0}),
        ] {
            let host = HostVideo::from_report(&report);
            assert_eq!(host, HostVideo::default());
            let panel = host.panel_video(Video::default());
            assert_eq!(panel.bandwidth_mbps, Some(10));
            assert_eq!(panel.fps, Some(60));
            assert_eq!(panel.constant_fps, Some(false));
            assert_eq!(panel.rotated, Some(false));
            assert!(panel.validate().is_ok());
        }
    }
}
