//! Host settings transactions only; decoding/presentation stay on their workers.
use super::transport::Attempt;
use super::{host_displays, host_video, profile::Video, session_ui};
use crate::window::Window;
use anyhow::{ensure, Result};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) struct HostSettings {
    config: host_video::Config,
    query_deadline: Option<Instant>,
    confirmation: Option<host_video::Confirmation>,
    pending: Option<Video>,
    requested_display: Option<(u32, u32)>,
    pending_output: Option<String>,
    outputs_deadline: Option<Instant>,
}

impl HostSettings {
    pub fn new(video: &Video, window: &Window) -> Self {
        window
            .native_hid_primary_ready
            .store(false, Ordering::Release);
        let pending = video.has_host_settings().then(|| video.clone());
        if pending.is_some() {
            session_ui::waiting(window);
        }
        Self {
            config: host_video::Config::default(),
            query_deadline: None,
            confirmation: None,
            pending,
            requested_display: None,
            pending_output: None,
            outputs_deadline: None,
        }
    }

    fn busy(&self) -> bool {
        self.confirmation.is_some() || self.pending.is_some() || self.pending_output.is_some()
    }

    pub fn connected(&mut self, attempt: &Attempt, window: &Window) -> Result<()> {
        self.query_outputs(attempt, window);
        self.query_deadline = Some(Instant::now() + QUERY_TIMEOUT);
        if attempt.send(&host_video::query()?).is_err() {
            crate::event_log::record(
                "WARN",
                "native_host_video_query_failed",
                serde_json::json!({}),
            );
        }
        Ok(())
    }

    pub fn receive(&mut self, bytes: &[u8], attempt: &Attempt, window: &Window) {
        self.query_deadline = None;
        if self.config.receive(bytes).is_err() {
            self.hid_primary_ready(attempt, window, false);
            self.pending = None;
            self.pending_output = None;
            crate::event_log::record(
                "WARN",
                "native_host_video_configuration_invalid",
                host_video::diagnostic(bytes),
            );
            if self.confirmation.is_none() {
                self.failed(window);
            }
            return;
        }

        window
            .native_displays
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .current = self.config.output_id().map(str::to_owned);
        session_ui::displays_changed(window);
        if let Some(id) = self.pending_output.take() {
            let result = (|| -> Result<()> {
                window
                    .native_displays
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .validate_selection(&id, attempt.hid_mouse_enabled())?;
                attempt.send(&self.config.select_output(&id)?)?;
                self.confirmation = Some(host_video::Confirmation::output(&id, Instant::now()));
                Ok(())
            })();
            crate::event_log::record(
                if result.is_ok() { "INFO" } else { "WARN" },
                "native_host_display_requested",
                serde_json::json!({"sent":result.is_ok()}),
            );
            if result.is_err() {
                self.failed(window);
                return;
            }
        }
        let observed = self.config.observed();
        crate::event_log::record("INFO", "native_host_video_configuration", observed.clone());
        *window
            .native_host_video
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = observed;
        if !self.ensure_hid_primary(attempt, window) {
            return;
        }
        if !self.busy() {
            session_ui::reported(window);
        }
        if self.confirmation.is_none() {
            self.send_pending(attempt, window);
        }
    }
    fn send_pending(&mut self, attempt: &Attempt, window: &Window) {
        let Some(next) = self.pending.take() else {
            return;
        };
        let size = window.native_display_size();
        match self.send(attempt, &next, size) {
            Ok(()) => {
                self.requested_display = Some(size);
                session_ui::waiting(window);
            }
            Err(_) => {
                crate::event_log::record(
                    "WARN",
                    "native_host_video_request_failed",
                    serde_json::json!({}),
                );
                self.failed(window);
            }
        }
    }

    pub fn poll(&mut self, attempt: &Attempt, window: &Window) -> Result<()> {
        if self
            .outputs_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.outputs_deadline = None;
            let mut displays = window
                .native_displays
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            displays.loading = false;
            displays.outputs.clear();
            drop(displays);
            self.hid_primary_ready(attempt, window, false);
            if attempt.hid_mouse_enabled() {
                self.pending = None;
                self.failed(window);
            }
            session_ui::displays_changed(window);
            crate::event_log::record(
                "WARN",
                "native_host_display_query_timeout",
                serde_json::json!({}),
            );
        }

        if let Some(verify) = self.confirmation.as_mut() {
            match verify.poll(&self.config, Instant::now()) {
                host_video::Verification::Query => {
                    if attempt.send(&host_video::query()?).is_err() {
                        self.confirmation = None;
                        self.pending = None;
                        session_ui::finished(window, false);
                    }
                }
                status @ (host_video::Verification::Confirmed
                | host_video::Verification::TimedOut) => {
                    let confirmed = status == host_video::Verification::Confirmed;
                    self.confirmation = None;
                    crate::event_log::record(
                        if confirmed { "INFO" } else { "WARN" },
                        if confirmed {
                            "native_host_video_confirmed"
                        } else {
                            "native_host_video_not_confirmed"
                        },
                        self.config.observed(),
                    );
                    session_ui::finished(window, confirmed);
                    if confirmed && self.ensure_hid_primary(attempt, window) {
                        self.send_pending(attempt, window);
                    } else if !confirmed {
                        self.pending = None;
                    }
                }
                host_video::Verification::Waiting => {}
            }
        }
        if self
            .query_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.query_deadline = None;
            self.pending = None;
            self.pending_output = None;
            crate::event_log::record(
                "WARN",
                "native_host_video_query_timeout",
                serde_json::json!({}),
            );
            self.failed(window);
        }
        Ok(())
    }

    fn hid_primary_ready(&self, attempt: &Attempt, window: &Window, ready: bool) {
        if !attempt.hid_mouse_enabled() {
            return;
        }
        let previous = window
            .native_hid_primary_ready
            .swap(ready, Ordering::AcqRel);
        if previous != ready {
            if previous {
                let _ = attempt.send(&crate::control::header(24, 0, 0, 0));
            }
            crate::event_log::record(
                if ready { "INFO" } else { "WARN" },
                "hid_primary_display_state",
                serde_json::json!({"ready":ready}),
            );
            session_ui::displays_changed(window);
        }
    }
    fn ensure_hid_primary(&mut self, attempt: &Attempt, window: &Window) -> bool {
        if !attempt.hid_mouse_enabled() {
            return true;
        }
        let primary = {
            let displays = window
                .native_displays
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            // Verified stock Windows host preserves EnumAdapters1/EnumOutputs
            // order. Both DXGI APIs specify primary first; other OSes are unknown.
            match displays.primary(self.config.windows_host(), self.config.output_id()) {
                host_displays::Primary::Ready => {
                    drop(displays);
                    self.hid_primary_ready(attempt, window, true);
                    return true;
                }
                host_displays::Primary::Waiting => None,
                host_displays::Primary::Select(id) => Some(id.to_owned()),
            }
        };
        self.hid_primary_ready(attempt, window, false);
        let Some(id) = primary else {
            let displays = window
                .native_displays
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let unavailable = self.config.ready() && !displays.loading;
            drop(displays);
            if unavailable {
                self.pending = None;
                self.failed(window);
            }
            return false;
        };
        if self.confirmation.is_some() {
            return false;
        }
        let result = self
            .config
            .select_output(&id)
            .and_then(|frame| attempt.send(&frame));
        crate::event_log::record(
            if result.is_ok() { "INFO" } else { "WARN" },
            "hid_primary_display_requested",
            serde_json::json!({"sent":result.is_ok()}),
        );
        if result.is_ok() {
            self.confirmation = Some(host_video::Confirmation::output(&id, Instant::now()));
            session_ui::waiting(window);
        } else {
            self.pending = None;
            self.failed(window);
        }
        false
    }

    fn query_outputs(&mut self, attempt: &Attempt, window: &Window) {
        self.hid_primary_ready(attempt, window, false);
        let sent = attempt
            .send(&host_displays::query().expect("Empty display query fits control frame"))
            .is_ok();
        self.outputs_deadline = sent.then(|| Instant::now() + QUERY_TIMEOUT);
        let mut displays = window
            .native_displays
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        displays.loading = sent;
        displays.hid_mouse = attempt.hid_mouse_enabled();
        if !sent {
            displays.outputs.clear();
        }
        drop(displays);
        session_ui::displays_changed(window);
        if !sent {
            crate::event_log::record(
                "WARN",
                "native_host_display_query_failed",
                serde_json::json!({}),
            );
        }
    }
    pub fn receive_outputs(&mut self, bytes: &[u8], attempt: &Attempt, window: &Window) {
        self.outputs_deadline = None;
        let result = host_displays::parse(bytes);
        let valid = result.is_ok();
        let mut displays = window
            .native_displays
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        displays.outputs = result.unwrap_or_default();
        displays.loading = false;
        let count = displays.outputs.len();
        drop(displays);
        crate::event_log::record(
            if valid { "INFO" } else { "WARN" },
            "native_host_displays_received",
            serde_json::json!({"valid":valid,"count":count}),
        );
        if self.ensure_hid_primary(attempt, window) && self.confirmation.is_none() {
            self.send_pending(attempt, window);
        }
        session_ui::displays_changed(window);
    }
    pub fn refresh_outputs(&mut self, attempt: &Attempt, window: &Window) -> Result<()> {
        ensure!(!self.busy(), "Waiting for the previous host request");
        self.query_outputs(attempt, window);
        attempt.send(&host_video::query()?)?;
        Ok(())
    }
    pub fn select_output(&mut self, attempt: &Attempt, window: &Window, id: &str) -> Result<()> {
        ensure!(
            !attempt.hid_mouse_enabled(),
            "Display switching is unavailable with HID mouse input"
        );
        ensure!(!self.busy(), "Waiting for the previous host request");
        let displays = window
            .native_displays
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        displays.validate_selection(id, attempt.hid_mouse_enabled())?;
        if displays.current.as_deref() == Some(id) {
            return Ok(());
        }
        drop(displays);
        // Read fresh settings before patching only output; never overwrite a
        // host-side change with an old cached complete configuration.
        attempt.send(&host_video::query()?)?;
        self.pending_output = Some(id.to_owned());
        self.query_deadline = Some(Instant::now() + QUERY_TIMEOUT);
        session_ui::waiting(window);
        Ok(())
    }

    pub fn apply(
        &mut self,
        attempt: &Attempt,
        window: &Window,
        video: &Video,
    ) -> Result<(u32, u32)> {
        ensure!(!self.busy(), "Waiting for the previous host request");
        let size = window.native_display_size();
        if video.has_host_settings() {
            if self.config.ready() {
                self.send(attempt, video, size)?;
            } else {
                self.pending = Some(video.clone());
                self.query_deadline = Some(Instant::now() + QUERY_TIMEOUT);
                attempt.send(&host_video::query()?)?;
            }
        }
        if self.busy() {
            session_ui::waiting(window);
        }
        Ok(size)
    }

    pub fn applied_locally(&mut self, size: (u32, u32)) {
        self.requested_display = Some(size);
    }

    pub fn result(&self, window: &Window, ok: bool) {
        if !self.busy() {
            session_ui::result(window, ok);
        }
    }

    pub fn resized(&mut self, attempt: &Attempt, window: &Window, video: &Video) -> Result<()> {
        let size = window.native_display_size();
        if self.requested_display == Some(size) || !self.config.ready() || self.busy() {
            return Ok(());
        }
        self.send(attempt, video, size)?;
        session_ui::waiting(window);
        self.requested_display = Some(size);
        Ok(())
    }

    fn failed(&self, window: &Window) {
        window
            .native_host_video_pending
            .store(false, Ordering::Release);
        session_ui::result(window, false);
    }

    fn send(&mut self, attempt: &Attempt, video: &Video, size: (u32, u32)) -> Result<()> {
        let frame = self.config.request(video, size)?;
        let confirmation = host_video::Confirmation::new(&frame, Instant::now())?;
        attempt.send(&frame)?;
        crate::event_log::record(
            "INFO",
            "native_host_video_requested",
            serde_json::json!({"fps":video.fps,"constant_fps":video.constant_fps,"bandwidth_mbps":video.bandwidth_mbps,"rotated":video.rotated,"resolution":video.resolution}),
        );
        self.confirmation = Some(confirmation);
        Ok(())
    }
}
