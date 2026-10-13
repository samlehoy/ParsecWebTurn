//! Snapshot-specific Parsec ABI state. Transport and decoding are separate:
//! initializing this object must never imply an established session.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::VecDeque;

#[derive(Default, Serialize)]
pub struct Backend {
    #[serde(skip)]
    pub connection_settings: Option<std::sync::Arc<crate::connection_settings::Manager>>,
    #[serde(skip)]
    relay_route: Option<crate::relay_fallback::Route>,
    #[serde(skip)]
    relay_pending: Option<std::time::Instant>,
    #[serde(skip)]
    relay_launching: Option<std::time::Instant>,
    #[serde(skip)]
    retiring_attempt: Option<crate::attempt::Attempt>,
    pub initialized: bool,
    pub stun_provider: crate::attempt::StunProvider,
    pub legacy_rsa_1024_enabled: bool,
    pub generation: u64,
    pub status: Option<i32>,
    pub video_protocol: Option<VideoProtocol>,
    pub idle_messages_discarded: u64,
    pub host_mode: i32,
    pub encode_latency: f32,
    pub control_frames_received: u64,
    pub media_ingress: crate::media_ingress::Ingress,
    pub input_availability: InputAvailability,
    pub attempt_failure: Option<crate::attempt::FailureStage>,
    #[cfg(any(test, feature = "diagnostics"))]
    pub attempt_diagnostic: Option<Value>,
    #[cfg(any(test, feature = "diagnostics"))]
    pub remote_begin_diagnostic: Option<Value>,
    #[cfg(any(test, feature = "diagnostics"))]
    pub previous_attempt_diagnostics: VecDeque<Value>,
    #[cfg(any(test, feature = "diagnostics"))]
    pub previous_attempts_omitted: u64,
    #[serde(skip)]
    pub guests: Vec<Value>,
    #[serde(skip)]
    pub me: Value,
    #[serde(skip)]
    pub attempt_id: String,
    #[serde(skip)]
    pub attempt_started: Option<std::time::Instant>,
    #[serde(skip)]
    pub native_attempt: Option<crate::attempt::Attempt>,
    #[serde(skip)]
    pub buffers: crate::buffers::Buffers,
    #[serde(skip)]
    events: VecDeque<Value>,
}

#[derive(Default, Serialize)]
pub struct InputAvailability {
    pub absolute_mouse_events_not_sent: u64,
    pub absolute_mouse_events_prepared: u64,
    pub unavailable_reason: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VideoProtocol {
    pub version: u32,
    pub message_size: u32,
    pub version_offset: u32,
    pub flag_offset: u32,
}

impl Backend {
    pub fn init(&mut self) {
        if !self.initialized {
            self.initialized = true;
            self.generation += 1;
            self.status = Some(-3); // X constructor in the audited parsec.js.
        }
    }

    pub fn destroy(&mut self) {
        self.clear_relay_fallback();
        self.retain_attempt_diagnostic();
        self.initialized = false;
        self.status = None;
        self.video_protocol = None;
        self.events.clear();
        self.buffers.clear();
        self.idle_messages_discarded = 0;
        self.host_mode = 0;
        self.encode_latency = 0.0;
        self.control_frames_received = 0;
        self.media_ingress = Default::default();
        self.input_availability = Default::default();
        self.guests.clear();
        self.me = Value::Null;
        self.attempt_id.clear();
        self.attempt_started = None;
    }

    fn clear_relay_fallback(&mut self) {
        self.relay_route = None;
        self.relay_pending = None;
        self.relay_launching = None;
        self.retiring_attempt = None;
    }

    pub fn route_for_new_attempt(&mut self) -> Option<crate::relay_fallback::Route> {
        let forced = self
            .connection_settings
            .as_ref()
            .is_some_and(|m| m.view().0.force_relay);
        if forced && self.relay_launching.is_some() {
            self.relay_launching = None;
            return self.relay_route.clone();
        }
        self.clear_relay_fallback();
        if forced {
            self.relay_route = Some(Default::default());
        }
        self.relay_route.clone()
    }

    /// Called before handling new user commands; never interfere with a stream.
    pub fn cancel_pending_relay_retry(&mut self) {
        if self.relay_pending.is_some() || self.relay_launching.is_some() {
            self.clear_relay_fallback();
        }
    }

    /// The GUI worker invokes the unchanged core's own Reconnect UI action.
    /// Only do so after the previous peer closed and its event was consumed.
    pub fn take_relay_reconnect(&mut self, core_ready: bool) -> Result<bool> {
        let Some(started) = self.relay_pending else {
            if self
                .relay_launching
                .is_some_and(|t| t.elapsed() >= std::time::Duration::from_secs(3))
            {
                self.clear_relay_fallback();
                self.fail_native_attempt(crate::attempt::FailureStage::Deadline)?;
            }
            return Ok(false);
        };
        if !self
            .connection_settings
            .as_ref()
            .is_some_and(|m| m.view().0.force_relay)
        {
            self.clear_relay_fallback();
            return Ok(false);
        }
        if started.elapsed() >= std::time::Duration::from_secs(3) {
            self.clear_relay_fallback();
            self.fail_native_attempt(crate::attempt::FailureStage::Deadline)?;
            return Ok(false);
        }
        if !core_ready
            || self.status != Some(-3)
            || !self.events.is_empty()
            || !self
                .retiring_attempt
                .as_ref()
                .is_some_and(|a| a.retry_cleanup_finished())
        {
            return Ok(false);
        }
        #[cfg(any(test, feature = "diagnostics"))]
        {
            self.attempt_diagnostic = self.retiring_attempt.as_ref().map(|a| a.snapshot());
        }
        self.retiring_attempt = None;
        self.relay_pending = None;
        self.relay_launching = Some(std::time::Instant::now());
        Ok(true)
    }

    pub fn require_initialized(&self) -> Result<()> {
        if !self.initialized {
            bail!("Parsec backend has not been initialized");
        }
        Ok(())
    }

    pub fn prepare_attempt(&mut self) {
        #[cfg(any(test, feature = "diagnostics"))]
        if self.attempt_diagnostic.is_some()
            || self.remote_begin_diagnostic.is_some()
            || self.attempt_failure.is_some()
        {
            if self.previous_attempt_diagnostics.len() == 4 {
                self.previous_attempt_diagnostics.pop_front();
                self.previous_attempts_omitted = self.previous_attempts_omitted.saturating_add(1);
            }
            self.previous_attempt_diagnostics.push_back(json!({
                "attempt":self.attempt_diagnostic,
                "failure":self.attempt_failure,
                "remote_begin":self.remote_begin_diagnostic
            }));
        }
        self.attempt_failure = None;
        #[cfg(any(test, feature = "diagnostics"))]
        {
            self.attempt_diagnostic = None;
            self.remote_begin_diagnostic = None;
        }
        self.events.clear();
        self.buffers.clear();
        self.guests.clear();
        self.me = Value::Null;
        self.host_mode = 0;
        self.encode_latency = 0.0;
        self.control_frames_received = 0;
        self.media_ingress = Default::default();
        self.input_availability = Default::default();
    }

    /// Include a live attempt without consuming, cancelling or pumping it.
    #[cfg(any(test, feature = "diagnostics"))]
    pub fn diagnostic(&self) -> Result<Value> {
        let mut value = serde_json::to_value(self)?;
        value["active_attempt_diagnostic"] = self
            .native_attempt
            .as_ref()
            .map(|a| a.snapshot())
            .unwrap_or(Value::Null);
        Ok(value)
    }

    #[cfg(test)]
    pub fn prepare_input(&mut self, message: &Value) -> Result<Option<bytes::Bytes>> {
        self.prepare_input_with_viewport(message, None)
    }
    pub fn prepare_input_with_viewport(
        &mut self,
        message: &Value,
        viewport: Option<crate::viewport::Viewport>,
    ) -> Result<Option<bytes::Bytes>> {
        match crate::control::input_with_viewport(message, viewport) {
            Ok(packet) => {
                if message["type"].as_i64() == Some(4)
                    && message["relative"].as_bool() == Some(false)
                {
                    self.input_availability.absolute_mouse_events_prepared = self
                        .input_availability
                        .absolute_mouse_events_prepared
                        .saturating_add(1);
                    self.input_availability.unavailable_reason = None;
                }
                Ok(Some(packet))
            }
            Err(error)
                if error
                    .downcast_ref::<crate::control::AbsoluteMouseUnavailable>()
                    .is_some() =>
            {
                self.input_availability.absolute_mouse_events_not_sent = self
                    .input_availability
                    .absolute_mouse_events_not_sent
                    .saturating_add(1);
                self.input_availability.unavailable_reason =
                    Some("absolute-mouse-requires-presented-video");
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub fn discard_idle_message(&mut self) -> Result<()> {
        self.require_initialized()?;
        // Audited h.W / clientSendMessage sends only if a transport exists
        // and status == 0. Preserve its idle no-op, not a fake delivery.
        if self.status == Some(0) {
            bail!("Live message transport is not implemented");
        }
        self.idle_messages_discarded = self.idle_messages_discarded.saturating_add(1);
        Ok(())
    }

    pub fn set_video_protocol(&mut self, protocol: VideoProtocol) -> Result<()> {
        self.require_initialized()?;
        // These offsets are used for 32-bit little-endian header fields in
        // parsec.js. Reject invalid metadata before a future decoder reads it.
        if protocol.message_size > 1024 * 1024
            || protocol
                .version_offset
                .checked_add(4)
                .is_none_or(|n| n > protocol.message_size)
            || protocol
                .flag_offset
                .checked_add(4)
                .is_none_or(|n| n > protocol.message_size)
        {
            bail!("invalid video protocol header layout");
        }
        if let Some(attempt) = &self.native_attempt {
            attempt.configure_video_protocol(protocol.clone());
        }
        self.video_protocol = Some(protocol);
        Ok(())
    }

    pub fn disconnect(&mut self, status: i32, state: i32) -> Result<()> {
        self.require_initialized()?;
        // The core acknowledges the intermediate cancelled connection before
        // Reconnect is dispatched. User cancellation is also observed by input.
        if self.relay_pending.is_some() && status != 0 {
            // Discard late errors/cleanup from the retired connection. Explicit
            // user commands cancel the pending retry before they reach the core.
            self.status = Some(-3);
            return Ok(());
        }
        if self.relay_pending.is_none() && self.relay_launching.is_none() {
            self.clear_relay_fallback();
        }
        let event = if state == 4 && self.status != Some(0) {
            Some(json!({
                "type": 7, "status": status, "state": state,
                "attemptID": self.attempt_id, "duration": 0
            }))
        } else if state == 8 && self.status == Some(0) {
            Some(
                json!({"type":7,"status":0,"state":state,"attemptID":self.attempt_id,
                "duration":self.attempt_started.map_or(0,|t|t.elapsed().as_secs())}),
            )
        } else {
            None
        };
        // Z() in the pinned adapter only closes transport/media for nonzero
        // status. A successful status update must not cancel the live attempt.
        if status != 0 {
            self.retain_attempt_diagnostic();
            self.events
                .retain(|event| !matches!(event["type"].as_i64(), Some(1 | 3)));
            self.buffers.clear();
        }
        self.status = Some(status);
        self.guests.clear();
        self.me = Value::Null;
        self.host_mode = 0;
        if let Some(event) = event {
            if self.events.len() >= 32 {
                bail!("backend event queue limit reached after disconnect cleanup");
            }
            self.events.push_back(event);
        }
        Ok(())
    }

    pub fn poll_event(&mut self) -> Option<Value> {
        self.events.pop_front()
    }

    fn retain_attempt_diagnostic(&mut self) {
        if let Some(mut attempt) = self.native_attempt.take() {
            if self.status == Some(0) {
                attempt.disconnect();
            }
            self.media_ingress = attempt.media_ingress();
            #[cfg(any(test, feature = "diagnostics"))]
            {
                self.attempt_diagnostic = Some(attempt.snapshot());
            }
        }
    }

    /// Expected connection failure is an app event, not a Wasmtime trap.
    pub fn fail_native_attempt(&mut self, stage: crate::attempt::FailureStage) -> Result<()> {
        let next = self
            .native_attempt
            .as_ref()
            .and_then(|a| a.relay_retry_target(stage));
        if let Some(next) = next {
            if self.status != Some(0) {
                let mut retiring = self
                    .native_attempt
                    .take()
                    .context("relay attempt missing")?;
                retiring.cancel();
                self.retiring_attempt = Some(retiring);
                self.relay_route = Some(next);
                self.relay_pending = Some(std::time::Instant::now());
                self.attempt_failure = Some(stage);
                self.events.clear();
                self.buffers.clear();
                self.status = Some(-3);
                // -3 is the pinned adapter's ordinary cancelled/idle status.
                // Final errors retain -6200; no fake success reaches the core.
                self.events.push_back(json!({"type":7,"status":-3,"state":4,"attemptID":self.attempt_id,"duration":0}));
                return Ok(());
            }
        }
        self.clear_relay_fallback();
        self.attempt_failure = Some(stage);
        let connected = self.status == Some(0);
        // Reserve space for the single failure event even if ICE events filled
        // the queue. No success or previously queued media can survive failure.
        self.events.clear();
        self.disconnect(-6200, 4)?;
        if connected {
            self.events.push_back(json!({"type":7,"status":-6200,"state":8,"attemptID":self.attempt_id,"duration":self.attempt_started.map_or(0,|t|t.elapsed().as_secs())}));
        }
        Ok(())
    }

    pub fn pump_native_events(&mut self) -> Result<()> {
        if let Err(error) = self.pump() {
            let stage = error
                .downcast_ref::<crate::attempt::FailureStage>()
                .copied()
                .unwrap_or(crate::attempt::FailureStage::InboundControl);
            // Malformed/unsupported inbound traffic fails this connection,
            // preserving its snapshot and notifying the guest, not trapping WASM.
            self.fail_native_attempt(stage)?;
        }
        Ok(())
    }

    fn pump(&mut self) -> Result<()> {
        if let Some(attempt) = &self.native_attempt {
            self.media_ingress = attempt.media_ingress();
            while self.events.len() < 32 {
                let Some(event) = attempt.pop_event() else {
                    break;
                };
                if event["type"] == 7 {
                    self.status =
                        Some(i32::try_from(event["status"].as_i64().ok_or_else(
                            || anyhow::anyhow!("native status event missing value"),
                        )?)?);
                }
                self.events.push_back(event);
            }
        }
        if self.status == Some(0) {
            self.clear_relay_fallback();
        }
        if self
            .native_attempt
            .as_ref()
            .is_some_and(|a| a.control_ready())
        {
            // Only control receipts enter this queue in configured sessions.
            // Media is counted/discarded directly by the transport reader.
            for _ in 0..32 {
                if self.events.len() >= 32 {
                    break;
                }
                let Some((channel, text, bytes)) =
                    self.native_attempt.as_ref().and_then(|a| a.pop_binary())
                else {
                    break;
                };
                self.handle_message(channel, text, bytes)?;
            }
        }
        Ok(())
    }

    pub(crate) fn handle_message(
        &mut self,
        channel: u16,
        text: bool,
        bytes: bytes::Bytes,
    ) -> Result<()> {
        if text || channel > 2 {
            return Err(crate::attempt::FailureStage::InboundChannel.into());
        }
        if channel != 0 {
            self.media_ingress.unavailable(channel, bytes.len());
            return Ok(());
        }
        let decoded =
            crate::control::decode(&bytes).context(crate::attempt::FailureStage::InboundControl)?;
        self.control_frames_received = self.control_frames_received.saturating_add(1);
        match decoded {
            crate::control::Message::Status(status) => {
                self.status = Some(status);
                self.events.push_back(json!({"type":7,"status":status,"state":8,"attemptID":self.attempt_id,"duration":self.attempt_started.map_or(0,|t|t.elapsed().as_secs())}));
            }
            crate::control::Message::EncodeLatency(value) => self.encode_latency = value,
            crate::control::Message::Event(event) => self.events.push_back(event),
            crate::control::Message::HostMode(value) => self.host_mode = value,
            crate::control::Message::Guests { list, me } => {
                self.guests = list;
                self.me = me;
            }
            crate::control::Message::Buffer { mut event, payload } => {
                let key = if let Some(range) = payload {
                    self.buffers.insert(bytes, range)?
                } else {
                    0
                };
                event["key"] = json!(key);
                self.events.push_back(event);
            }
            crate::control::Message::Ignored => {}
        }
        Ok(())
    }

    pub fn peek_event(&self) -> Option<&Value> {
        self.events.front()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn retired_cleanup_cannot_replace_pending_retry_with_a_late_error() {
        let mut backend = super::Backend::default();
        backend.init();
        backend.relay_pending = Some(std::time::Instant::now());
        backend.disconnect(-6200, 4).unwrap();
        assert_eq!(backend.status, Some(-3));
        assert!(backend.poll_event().is_none());
        backend.cancel_pending_relay_retry();
        backend.disconnect(-6200, 4).unwrap();
        assert_eq!(backend.poll_event().unwrap()["status"], -6200);

        backend.relay_launching =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
        assert!(!backend.take_relay_reconnect(true).unwrap());
        assert!(backend.relay_launching.is_none());
        assert_eq!(backend.poll_event().unwrap()["status"], -6200);
    }

    #[test]
    fn ordinary_connections_and_user_cancellation_discard_pending_fallbacks() {
        let mut backend = super::Backend::default();
        backend.init();
        backend.relay_route = Some(Default::default());
        backend.relay_pending = Some(std::time::Instant::now());
        assert!(backend.route_for_new_attempt().is_none());
        assert!(backend.relay_pending.is_none());
        assert!(backend.relay_route.is_none());
        backend.relay_route = Some(Default::default());
        backend.relay_launching = Some(std::time::Instant::now());
        backend.cancel_pending_relay_retry();
        assert!(backend.relay_launching.is_none());
        assert!(backend.relay_route.is_none());
        backend.relay_pending = Some(std::time::Instant::now());
        assert!(!backend.take_relay_reconnect(true).unwrap()); // Force is absent
        assert!(backend.relay_pending.is_none());
    }
    #[test]
    fn successful_status_update_preserves_media_but_disconnect_retires_it() {
        let mut backend = Backend::default();
        backend.init();
        backend.status = Some(0);
        let key = backend
            .buffers
            .insert(bytes::Bytes::from_static(b"cursor"), 0..6)
            .unwrap();
        backend.events.push_back(json!({"type":1,"key":key}));
        backend.disconnect(0, 0).unwrap();
        assert_eq!(backend.buffers.size(key), 6);
        assert_eq!(backend.peek_event().unwrap()["type"], 1);
        backend.disconnect(-3, 8).unwrap();
        assert_eq!(backend.buffers.size(key), 0);
        let event = backend.poll_event().unwrap();
        assert_eq!(event["type"], 7);
        assert_eq!(event["state"], 8);
        assert_eq!(event["status"], 0);
        assert_eq!(backend.status, Some(-3));
    }
    #[test]
    fn unavailable_absolute_mouse_does_not_fail_connection_or_claim_delivery() {
        let mut backend = Backend::default();
        backend.init();
        backend.status = Some(0);
        assert!(backend
            .prepare_input(&json!({"type":4,"relative":false,"x":123,"y":456}))
            .unwrap()
            .is_none());
        assert_eq!(backend.status, Some(0));
        assert_eq!(backend.input_availability.absolute_mouse_events_not_sent, 1);
        let packet = backend
            .prepare_input(&json!({"type":4,"relative":true,"x":-3,"y":4}))
            .unwrap()
            .unwrap();
        assert_eq!(packet, crate::control::header(3, 1, -3, 4));
        backend.prepare_attempt();
        assert_eq!(backend.input_availability.absolute_mouse_events_not_sent, 0);
    }
    #[test]
    fn malformed_input_is_not_silently_classified_as_unavailable() {
        let mut backend = Backend::default();
        backend.init();
        for message in [
            json!({"type":4,"relative":false,"x":"private-coordinate","y":0}),
            json!({"type":4,"relative":false,"x":2147483648_i64,"y":0}),
            json!({"type":4,"relative":0,"x":0,"y":0}),
            json!({"type":999}),
        ] {
            assert!(backend.prepare_input(&message).is_err());
        }
        assert_eq!(backend.input_availability.absolute_mouse_events_not_sent, 0);
    }
    #[test]
    fn unavailable_input_counter_saturates_and_diagnostics_omit_coordinates() {
        let mut backend = Backend::default();
        backend.init();
        backend.input_availability.absolute_mouse_events_not_sent = u64::MAX;
        backend
            .prepare_input(&json!({"type":4,"relative":false,"x":123456789,"y":987654321}))
            .unwrap();
        assert_eq!(
            backend.input_availability.absolute_mouse_events_not_sent,
            u64::MAX
        );
        let report = backend.diagnostic().unwrap().to_string();
        assert!(!report.contains("123456789") && !report.contains("987654321"));
    }

    #[test]
    fn retry_history_keeps_failure_and_is_bounded_without_ids() {
        let mut b = Backend::default();
        b.init();
        b.attempt_id = "private-attempt-sentinel".into();
        for index in 0..6 {
            b.attempt_diagnostic = Some(json!({"remote_candidates":index}));
            b.attempt_failure = Some(crate::attempt::FailureStage::Deadline);
            b.prepare_attempt();
        }
        let value = b.diagnostic().unwrap();
        assert_eq!(
            value["previous_attempt_diagnostics"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        assert_eq!(value["previous_attempts_omitted"], 2);
        assert_eq!(
            value["previous_attempt_diagnostics"][0]["attempt"]["remote_candidates"],
            2
        );
        assert_eq!(
            value["previous_attempt_diagnostics"][3]["failure"],
            "deadline"
        );
        assert!(value["active_attempt_diagnostic"].is_null());
        assert!(!value.to_string().contains("private-attempt-sentinel"));
    }
    #[test]
    fn transport_failure_replaces_pending_events_and_cleans_buffers() {
        for status in [20, 0] {
            let mut b = Backend::default();
            b.init();
            b.status = Some(status);
            let key = b
                .buffers
                .insert(bytes::Bytes::from_static(b"unread"), 0..6)
                .unwrap();
            for _ in 0..32 {
                b.events.push_back(json!({"type":2}));
            }
            b.fail_native_attempt(crate::attempt::FailureStage::RemoteCandidate)
                .unwrap();
            assert_eq!(b.status, Some(-6200));
            assert_eq!(b.buffers.size(key), 0);
            let event = b.poll_event().unwrap();
            assert_eq!(event["status"], -6200);
            assert_eq!(event["state"], if status == 0 { 8 } else { 4 });
            assert!(b.poll_event().is_none());
            assert_eq!(
                serde_json::to_value(&b).unwrap()["attempt_failure"],
                "remote-candidate"
            );
            b.prepare_attempt();
            assert!(b.attempt_failure.is_none());
        }
    }
    use super::*;

    #[test]
    fn idle_input_never_claims_a_live_message_delivery() {
        let mut b = Backend::default();
        assert!(b.discard_idle_message().is_err());
        b.init();
        b.discard_idle_message().unwrap();
        assert_eq!(b.idle_messages_discarded, 1);
        assert_eq!(b.status, Some(-3));
        b.status = Some(0);
        assert!(b.discard_idle_message().is_err());
        assert_eq!(b.idle_messages_discarded, 1);
        b.destroy();
        assert_eq!(b.idle_messages_discarded, 0);
    }

    #[test]
    fn lifecycle_is_idempotent_and_reinitialization_resets_state() {
        let mut b = Backend::default();
        assert!(b.require_initialized().is_err());
        b.init();
        b.init();
        assert_eq!(b.generation, 1);
        assert_eq!(b.status, Some(-3));
        b.disconnect(12, 4).unwrap();
        assert_eq!(b.poll_event().unwrap()["status"], 12);
        b.destroy();
        assert!(b.poll_event().is_none());
        assert_eq!(b.status, None);
        b.init();
        assert_eq!(b.generation, 2);
        assert_eq!(b.status, Some(-3));
    }

    #[test]
    fn metadata_rejects_overflow_and_out_of_header_fields() {
        let mut b = Backend::default();
        b.init();
        let good = VideoProtocol {
            version: 1,
            message_size: 16,
            version_offset: 0,
            flag_offset: 12,
        };
        b.set_video_protocol(good.clone()).unwrap();
        let bad = VideoProtocol {
            flag_offset: u32::MAX,
            ..good.clone()
        };
        assert!(b.set_video_protocol(bad).is_err());
        assert_eq!(b.video_protocol, Some(good));
    }
    #[test]
    fn guest_handles_expire_across_attempt_and_backend_lifecycles() {
        let mut b = Backend::default();
        b.init();
        let first = b
            .buffers
            .insert(bytes::Bytes::from_static(b"one"), 0..3)
            .unwrap();
        b.prepare_attempt();
        assert_eq!(b.buffers.size(first), 0);
        let second = b
            .buffers
            .insert(bytes::Bytes::from_static(b"two"), 0..3)
            .unwrap();
        b.disconnect(-3, 4).unwrap();
        assert_eq!(b.buffers.size(second), 0);
        b.destroy();
        b.init();
        let third = b
            .buffers
            .insert(bytes::Bytes::from_static(b"three"), 0..5)
            .unwrap();
        assert!(first < second && second < third);
        assert_eq!(b.buffers.size(first), 0);
    }
    #[test]
    fn full_event_queue_cannot_prevent_disconnect_buffer_cleanup() {
        let mut b = Backend::default();
        b.init();
        let key = b
            .buffers
            .insert(bytes::Bytes::from_static(b"unread"), 0..6)
            .unwrap();
        for _ in 0..32 {
            b.events.push_back(json!({"type":2}));
        }
        assert!(b.disconnect(-3, 4).is_err());
        assert_eq!(b.buffers.size(key), 0);
        assert_eq!(b.status, Some(-3));
    }
}
