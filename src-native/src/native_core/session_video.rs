//! Host settings transactions only; decoding/presentation stay on their workers.
use super::transport::Attempt;
use super::{host_video, profile::Video, session_ui};
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
}

impl HostSettings {
    pub fn new(video: &Video, window: &Window) -> Self {
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
        }
    }

    fn busy(&self) -> bool {
        self.confirmation.is_some() || self.pending.is_some()
    }

    pub fn connected(&mut self, attempt: &Attempt) -> Result<()> {
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
            self.pending = None;
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

        let observed = self.config.observed();
        crate::event_log::record("INFO", "native_host_video_configuration", observed.clone());
        *window
            .native_host_video
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = observed;
        if !self.busy() {
            session_ui::reported(window);
        }
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
        if let Some(verify) = self.confirmation.as_mut() {
            match verify.poll(&self.config, Instant::now()) {
                host_video::Verification::Query => {
                    if attempt.send(&host_video::query()?).is_err() {
                        self.confirmation = None;
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
            crate::event_log::record(
                "WARN",
                "native_host_video_query_timeout",
                serde_json::json!({}),
            );
            self.failed(window);
        }
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
