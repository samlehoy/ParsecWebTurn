//! Bounded compressed-video queue; decoder resources belong to one worker.
use serde::Serialize;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

// Allow bounded headroom while Media Foundation initializes its first device.
pub const MAX_FRAMES: usize = 32;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

/// CPU wall-clock durations only. No inference about GPU completion/scanout.
#[derive(Clone, Default, Serialize)]
pub struct Timing {
    pub count: u64,
    pub latest_ms: f64,
    pub average_ms: f64,
    pub peak_ms: f64,
    pub total_ns: u128,
}
impl Timing {
    pub fn record(&mut self, elapsed: Duration) {
        let ms = elapsed.as_secs_f64() * 1000.;
        self.count = self.count.saturating_add(1);
        self.latest_ms = ms;
        self.average_ms += (ms - self.average_ms) / self.count as f64;
        self.peak_ms = self.peak_ms.max(ms);
        self.total_ns = self.total_ns.saturating_add(elapsed.as_nanos());
    }
}

#[derive(Clone, Default, Serialize)]
pub struct LocalTiming {
    pub decoder_setup: Timing,
    pub compressed_wait: Timing,
    pub decoder_service: Timing,
    pub decoder_residence: Timing,
    pub surface_dispatch: Timing,
    pub presentation_prepare: Timing,
    pub present_call: Timing,
    /// Interval between accepted Present calls; not GPU/scanout timing.
    pub present_interval: Timing,
    /// Time spent servicing decoded output inside the decoder feed callback.
    pub output_service_ns: u128,
}

#[derive(Clone, Copy)]
struct PendingInput {
    sequence: usize,
    timestamp: i64,
    at: Instant,
}
/// Correlate native decoder outputs without retaining compressed bytes/surfaces.
/// Missing/evicted/ambiguous timestamps remain unknown, never FIFO guesses.
pub struct InFlight {
    slots: [Option<PendingInput>; 64],
    next: usize,
}
impl Default for InFlight {
    fn default() -> Self {
        Self {
            slots: [None; 64],
            next: 0,
        }
    }
}
impl InFlight {
    pub fn begin(&mut self, timestamp: i64) {
        self.slots[self.next % 64] = Some(PendingInput {
            sequence: self.next,
            timestamp,
            at: Instant::now(),
        });
        self.next = self.next.wrapping_add(1);
    }
    pub fn ready(
        &mut self,
        sequence: Option<usize>,
        timestamp: Option<i64>,
        now: Instant,
    ) -> Option<Duration> {
        if let Some(sequence) = sequence {
            let slot = &mut self.slots[sequence % 64];
            if slot.is_some_and(|p| p.sequence == sequence) {
                return now.checked_duration_since(slot.take()?.at);
            }
            return None;
        }
        let mut found = None;
        for (index, value) in self.slots.iter().enumerate() {
            if value.is_some_and(|p| timestamp.is_some_and(|t| p.timestamp == t)) {
                if found.is_some() {
                    return None;
                }
                found = Some(index);
            }
        }
        let input = self.slots[found?].take()?;
        now.checked_duration_since(input.at)
    }
}

/// Do not discard compressed references. Only omit obsolete display updates
/// when at least two later compressed messages already await decoding.
pub fn skip_display_for_backlog(bud: bool, queued: usize) -> bool {
    bud && queued >= 2
}

#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub local_timing: LocalTiming,
    pub presentation_mode: Option<&'static str>,
    pub display_updates_skipped: u64,
    pub decoded_bit_depth: Option<u8>,
    pub decoded_chroma: Option<&'static str>,
    /// RGB swap-chain precision, not the monitor/link's physical bit depth.
    pub display_rgb_bit_depth: Option<u8>,
    pub display_output_fallback: Option<String>,
    pub codec: Option<crate::video_stream::Codec>,
    pub enabled: bool,
    pub decoder_initialized: bool,
    pub low_latency_request_accepted: bool,
    pub mmcss_registered: bool,
    pub decoder: Option<&'static str>,
    pub decoder_fallback: Option<String>,
    pub zero_copy_status: Option<String>,
    pub renderer: Option<&'static str>,
    pub adapter: Option<String>,
    pub gpu_surface_output: bool,
    pub synthetic_pixel_variation_verified: bool,
    pub hardware_decode: Option<bool>,
    /// Observed on the latest decoded output, not a capability/configuration flag.
    pub d3d11_decoder_surface: Option<bool>,
    pub frames_queued: u64,
    pub frames_dropped: u64,
    pub frames_submitted: u64,
    pub frames_decoded: u64,
    pub frames_presented: u64,
    pub input_views_created: u64,
    pub output_views_created: u64,
    pub auto_processing_disabled: bool,
    pub presentations_busy: u64,
    pub presentations_not_visible: u64,
    pub present_max_us: u64,
    pub decode_feed_max_us: u64,
    pub queue_peak_depth: usize,
    pub queue_depth: usize,
    pub queue_bytes: usize,
    pub waiting_for_idr: bool,
    pub recovering: bool,
    pub color_matrix: Option<u32>,
    pub nominal_range: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub failure_stage: Option<&'static str>,
    pub failure_hresult: Option<String>,
    pub worker_finished: bool,
    pub resources_released: bool,
}

impl Snapshot {
    pub fn hardware_decode_status(&self) -> &'static str {
        match (self.hardware_decode, self.d3d11_decoder_surface) {
            (Some(true), _) => "Yes",
            (Some(false), _) => "No",
            (None, Some(true)) => "Likely (D3D11 decoder surface)",
            _ => "Not reported",
        }
    }
}

pub struct Frame {
    /// Clockwise display quarter-turns, carried with compressed-frame identity.
    pub rotation: u8,
    pub queued_at: Instant,
    pub codec: crate::video_stream::Codec,
    pub bytes: Vec<u8>,
    pub idr: bool,
    pub parameters: bool,
    pub timestamp_100ns: i64,
}

pub struct Queue {
    pub bud_catch_up: bool,
    pub report: Snapshot,
    pub stopped: bool,
    frames: VecDeque<Frame>,
    free_buffers: Vec<Vec<u8>>,
    free_bytes: usize,
    pub reset_generation: u64,
    recovery_started: Option<Instant>,
    recovery_attempts: u8,
    request_pending: bool,
    last_request: Option<Instant>,
}

impl Default for Queue {
    fn default() -> Self {
        Self {
            bud_catch_up: false,
            report: Snapshot {
                waiting_for_idr: true,
                ..Default::default()
            },
            stopped: false,
            frames: VecDeque::new(),
            free_buffers: Vec::new(),
            free_bytes: 0,
            reset_generation: 0,
            recovery_started: None,
            recovery_attempts: 0,
            request_pending: false,
            last_request: None,
        }
    }
}

impl Queue {
    #[cfg(test)]
    pub fn submit(
        &mut self,
        bytes: &[u8],
        idr: bool,
        parameters: bool,
        timestamp_100ns: i64,
    ) -> bool {
        self.submit_codec(
            bytes.to_vec(),
            idr,
            parameters,
            timestamp_100ns,
            crate::video_stream::Codec::H264,
        )
    }

    pub fn submit_codec(
        &mut self,
        bytes: Vec<u8>,
        idr: bool,
        parameters: bool,
        timestamp_100ns: i64,
        codec: crate::video_stream::Codec,
    ) -> bool {
        if self.report.codec.is_some_and(|previous| previous != codec) {
            self.fail("video-unexpected-codec-change", None);
            return false;
        }
        if self.stopped
            || self.report.failure_stage.is_some()
            || (self.report.waiting_for_idr && !idr && !parameters)
        {
            self.report.frames_dropped = self.report.frames_dropped.saturating_add(1);
            self.recycle(bytes);
            return false;
        }
        if bytes.len() > crate::control::MAX_CHANNEL_MESSAGE {
            self.fail("video-input-too-large", None);
            self.report.frames_dropped += 1;
            return false;
        }
        if self.frames.len() >= MAX_FRAMES
            || self.report.queue_bytes.saturating_add(bytes.len()) > MAX_BYTES
        {
            // Following deltas are unusable after a missing reference picture.
            self.recover(Instant::now());
            self.report.frames_dropped = self.report.frames_dropped.saturating_add(1);
            return false;
        }
        if idr {
            self.report.waiting_for_idr = false;
        }
        self.report.frames_queued = self.report.frames_queued.saturating_add(1);
        self.report.codec = Some(codec);
        self.report.queue_bytes += bytes.len();
        self.frames.push_back(Frame {
            rotation: 0,
            queued_at: Instant::now(),
            codec,
            bytes,
            idr,
            parameters,
            timestamp_100ns,
        });
        self.report.queue_depth = self.frames.len();
        self.report.queue_peak_depth = self.report.queue_peak_depth.max(self.frames.len());
        true
    }

    pub fn set_last_rotation(&mut self, rotation: u8) {
        if let Some(frame) = self.frames.back_mut() {
            frame.rotation = rotation;
        }
    }

    pub fn pop(&mut self) -> Option<Frame> {
        let frame = self.frames.pop_front()?;
        self.report.queue_bytes -= frame.bytes.len();
        self.report.queue_depth = self.frames.len();
        self.report
            .local_timing
            .compressed_wait
            .record(frame.queued_at.elapsed());
        Some(frame)
    }

    pub fn take_buffer(&mut self) -> Vec<u8> {
        let buffer = self.free_buffers.pop().unwrap_or_default();
        self.free_bytes -= buffer.capacity();
        buffer
    }

    pub fn recycle(&mut self, mut buffer: Vec<u8>) {
        if !self.stopped
            && self.report.failure_stage.is_none()
            && self.free_buffers.len() < MAX_FRAMES
            && self.free_bytes.saturating_add(buffer.capacity()) <= MAX_BYTES
        {
            buffer.clear();
            self.free_bytes += buffer.capacity();
            self.free_buffers.push(buffer);
        }
    }

    pub fn recover(&mut self, now: Instant) -> bool {
        if self.stopped || self.report.failure_stage.is_some() {
            return false;
        }
        if self.recovery_attempts >= 3 {
            self.fail("video-recovery-exhausted", None);
            return false;
        }
        self.recovery_attempts += 1;
        self.reset_generation += 1;
        self.clear();
        self.report.waiting_for_idr = true;
        self.report.decoder_initialized = false;
        self.report.recovering = true;
        self.recovery_started = Some(now);
        self.request_pending = true;
        true
    }

    /// A valid stream format change is not a decoder failure. Preserve queued
    /// compressed frames, especially an already received random-access picture,
    /// and keep the existing keyframe deadline/rate limit if one is needed.
    pub fn begin_format_change(&mut self, now: Instant, needs_keyframe: bool) {
        self.report.decoder_initialized = false;
        if needs_keyframe {
            self.report.waiting_for_idr = true;
            self.report.recovering = true;
            self.recovery_started = Some(now);
            self.request_pending = true;
        }
    }

    pub fn check_recovery(&mut self, now: Instant) {
        if self
            .recovery_started
            .is_some_and(|start| now.duration_since(start) >= Duration::from_secs(5))
        {
            self.fail("video-recovery-timeout", None);
        }
    }

    pub fn take_keyframe_request(&mut self, now: Instant) -> Option<crate::video_stream::Codec> {
        self.check_recovery(now);
        if self.stopped
            || self.report.failure_stage.is_some()
            || !self.request_pending
            || self
                .last_request
                .is_some_and(|last| now.duration_since(last) < Duration::from_secs(1))
        {
            return None;
        }
        let codec = self.report.codec?;
        self.request_pending = false;
        self.last_request = Some(now);
        Some(codec)
    }

    pub fn recovered(&mut self) {
        self.report.waiting_for_idr = false;
        self.recovery_started = None;
        self.request_pending = false;
        self.report.recovering = false;
    }

    pub fn request_failed(&mut self, generation: u64) {
        if !self.stopped && self.report.recovering && self.reset_generation == generation {
            self.fail("video-recovery-request-failed", None);
        }
    }

    pub fn fail(&mut self, stage: &'static str, hresult: Option<String>) {
        if self.report.failure_stage.is_none() {
            crate::event_log::record(
                "ERROR",
                "video_stopped",
                serde_json::json!({"stage":stage,"hresult":hresult}),
            );
            self.report.failure_stage = Some(stage);
            self.report.failure_hresult = hresult;
        }
        self.clear();
        self.request_pending = false;
        self.recovery_started = None;
        self.report.recovering = false;
    }

    pub fn clear(&mut self) {
        self.report.frames_dropped = self
            .report
            .frames_dropped
            .saturating_add(self.frames.len() as u64);
        while let Some(frame) = self.frames.pop_front() {
            self.recycle(frame.bytes);
        }
        if self.stopped || self.report.failure_stage.is_some() {
            self.free_buffers.clear();
            self.free_bytes = 0;
        }
        self.report.queue_depth = 0;
        self.report.queue_bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_timing_is_bounded_and_measures_queue_age_without_sleeping() {
        let mut q = Queue::default();
        assert!(q.submit(&[1], true, false, 0));
        q.frames.front_mut().unwrap().queued_at = Instant::now() - Duration::from_millis(20);
        q.pop().unwrap();
        assert_eq!(q.report.local_timing.compressed_wait.count, 1);
        assert!(q.report.local_timing.compressed_wait.latest_ms >= 20.);
        let mut timing = Timing::default();
        timing.record(Duration::from_millis(2));
        timing.record(Duration::from_millis(6));
        assert_eq!(
            (timing.latest_ms, timing.average_ms, timing.peak_ms),
            (6., 4., 6.)
        );
        assert_eq!(timing.total_ns, 8_000_000);
        assert!(std::mem::size_of::<LocalTiming>() <= 512);
        assert!(std::mem::size_of::<InFlight>() <= 4096);
    }

    #[test]
    fn output_timing_matches_identity_not_fifo_and_forgets_old_slots() {
        let mut pending = InFlight::default();
        pending.begin(100);
        pending.begin(200);
        let now = Instant::now();
        assert!(pending.ready(None, Some(200), now).is_some());
        assert!(pending.ready(Some(0), None, now).is_some());
        assert!(pending.ready(Some(0), None, now).is_none());
        pending.begin(300);
        pending.begin(300);
        assert!(pending.ready(None, Some(300), Instant::now()).is_none());
        for n in 0..1000 {
            pending.begin(n);
        }
        assert!(pending.ready(None, Some(1), Instant::now()).is_none());
        assert!(pending.ready(None, Some(999), Instant::now()).is_some());
    }

    #[test]
    fn burst_catch_up_decodes_every_reference_but_reduces_display_work() {
        fn model(bud: bool, count: usize, render_us: u64) -> (Vec<i64>, u64, usize) {
            let mut q = Queue::default();
            for index in 0..count {
                assert!(q.submit(&[index as u8], index == 0, false, index as i64));
            }
            let mut decoded = Vec::new();
            let mut elapsed = 0;
            let mut displayed = 0;
            while let Some(frame) = q.pop() {
                decoded.push(frame.timestamp_100ns);
                elapsed += 2_000; // Same modeled decode cost for every reference.
                if !skip_display_for_backlog(bud, q.report.queue_depth) {
                    elapsed += render_us;
                    displayed += 1;
                }
            }
            (decoded, elapsed, displayed)
        }
        let baseline = model(false, 24, 4_000);
        let catch_up = model(true, 24, 4_000);
        assert_eq!(baseline.0, catch_up.0);
        assert_eq!((baseline.1, baseline.2), (144_000, 24));
        assert_eq!((catch_up.1, catch_up.2), (56_000, 2));
        // No claimed improvement when display work is free or no burst exists.
        assert_eq!(model(false, 24, 0).1, model(true, 24, 0).1);
        assert_eq!(model(false, 1, 4_000), model(true, 1, 4_000));
        assert!(!skip_display_for_backlog(false, MAX_FRAMES));
        eprintln!("synthetic burst: baseline={}us/{} display submissions; catch-up={}us/{}; all {} references decoded", baseline.1, baseline.2, catch_up.1, catch_up.2, catch_up.0.len());
    }
    #[test]
    fn late_refresh_failures_cannot_stop_a_new_or_recovered_generation() {
        let now = Instant::now();
        let mut q = Queue::default();
        assert!(q.submit(&[1], true, false, 0));
        assert!(q.recover(now));
        let old = q.reset_generation;
        assert!(q.recover(now + Duration::from_secs(1)));
        q.request_failed(old);
        assert_eq!(q.report.failure_stage, None);
        q.recovered();
        q.request_failed(q.reset_generation);
        assert_eq!(q.report.failure_stage, None);
        assert!(q.recover(now + Duration::from_secs(2)));
        q.request_failed(q.reset_generation);
        assert_eq!(
            q.report.failure_stage,
            Some("video-recovery-request-failed")
        );
    }
    #[test]
    fn format_changes_keep_queued_keys_and_do_not_spend_error_retries() {
        let mut queue = Queue::default();
        let now = Instant::now();
        assert!(queue.submit(&[1], true, true, 0));
        for _ in 0..8 {
            queue.begin_format_change(now, false);
        }
        assert!(queue.pop().unwrap().idr);
        assert_eq!(queue.reset_generation, 0);
        assert_eq!(queue.recovery_attempts, 0);
        assert!(queue.submit(&[2], true, true, 1));
        queue.begin_format_change(now, true);
        assert!(queue.take_keyframe_request(now).is_some());
        assert!(queue.pop().unwrap().idr);
        queue.recovered();
        assert!(queue.submit(&[3], false, false, 2));
        queue.begin_format_change(now, true);
        // A second boundary still respects the existing keyframe rate limit.
        assert!(queue.take_keyframe_request(now).is_none());
        queue.check_recovery(now + Duration::from_secs(5));
        assert_eq!(queue.report.failure_stage, Some("video-recovery-timeout"));
    }

    #[test]
    fn recovery_is_rate_limited_bounded_and_stops_on_timeout_or_cancellation() {
        let now = Instant::now();
        let mut q = Queue::default();
        assert!(q.submit(&[1], true, false, 0));
        assert!(q.recover(now));
        assert_eq!(
            q.take_keyframe_request(now),
            Some(crate::video_stream::Codec::H264)
        );
        assert_eq!(q.take_keyframe_request(now), None);
        assert!(q.recover(now + Duration::from_millis(10)));
        assert_eq!(
            q.take_keyframe_request(now + Duration::from_millis(999)),
            None
        );
        assert!(q
            .take_keyframe_request(now + Duration::from_secs(1))
            .is_some());
        q.recovered();
        assert!(q.recover(now + Duration::from_secs(2)));
        assert!(!q.recover(now + Duration::from_secs(3)));
        assert_eq!(q.report.failure_stage, Some("video-recovery-exhausted"));
        assert_eq!(q.take_keyframe_request(now + Duration::from_secs(10)), None);

        let mut q = Queue::default();
        assert!(q.submit(&[1], true, false, 0));
        assert!(q.recover(now));
        q.check_recovery(now + Duration::from_secs(5));
        assert_eq!(q.report.failure_stage, Some("video-recovery-timeout"));
        assert!(!q.report.recovering);
        let mut q = Queue {
            stopped: true,
            ..Default::default()
        };
        assert!(!q.recover(now));
        assert_eq!(q.take_keyframe_request(now), None);
    }
    #[test]
    fn compressed_buffers_are_reused_only_after_the_frame_is_consumed() {
        let mut q = Queue::default();
        let data = vec![7; 1024];
        let pointer = data.as_ptr();
        assert!(q.submit_codec(data, true, false, 0, crate::video_stream::Codec::H264));
        assert!(q.take_buffer().is_empty());
        let frame = q.pop().unwrap();
        assert_eq!(frame.bytes.as_ptr(), pointer);
        q.recycle(frame.bytes);
        let reused = q.take_buffer();
        assert_eq!(reused.as_ptr(), pointer);
        assert!(reused.is_empty());
        q.recycle(reused);
        q.stopped = true;
        q.clear();
        assert!(q.free_buffers.is_empty());
        assert_eq!(q.free_bytes, 0);
    }
    #[test]
    fn queued_frames_preserve_codec_and_refuse_a_midstream_change() {
        use crate::video_stream::Codec;
        let mut q = Queue::default();
        assert!(q.submit_codec(vec![1], false, true, 0, Codec::Hevc));
        assert_eq!(q.pop().unwrap().codec, Codec::Hevc);
        assert!(q.submit_codec(vec![2], true, false, 1, Codec::Hevc));
        assert!(!q.submit_codec(vec![3], true, false, 2, Codec::H264));
        assert_eq!(
            q.report.failure_stage,
            Some("video-unexpected-codec-change")
        );
        assert_eq!(q.report.queue_depth, 0);
    }
    #[test]
    fn gpu_output_alone_does_not_establish_hardware_decoding() {
        let mut snapshot = Snapshot {
            decoder_initialized: true,
            gpu_surface_output: true,
            ..Default::default()
        };
        assert_eq!(snapshot.hardware_decode_status(), "Not reported");
        snapshot.d3d11_decoder_surface = Some(false);
        assert_eq!(snapshot.hardware_decode_status(), "Not reported");
        snapshot.d3d11_decoder_surface = Some(true);
        assert_eq!(
            snapshot.hardware_decode_status(),
            "Likely (D3D11 decoder surface)"
        );
        assert_eq!(snapshot.hardware_decode, None);
        snapshot.hardware_decode = Some(false);
        assert_eq!(snapshot.hardware_decode_status(), "No");
    }
    #[test]
    fn overflow_discards_dependencies_and_waits_for_a_new_keyframe() {
        let mut q = Queue::default();
        for _ in 0..MAX_FRAMES {
            assert!(q.submit(&[1, 2, 3], true, false, 0));
        }
        assert!(!q.submit(&[4], false, false, 0));
        assert_eq!(q.report.failure_stage, None);
        assert!(q.report.recovering);
        assert!(q.report.waiting_for_idr);
        assert_eq!(q.reset_generation, 1);
        assert_eq!(q.report.queue_depth, 0);
        assert_eq!(q.report.queue_bytes, 0);
        assert_eq!(q.report.frames_dropped, MAX_FRAMES as u64 + 1);
        assert!(!q.submit(&[5], false, false, 0));
        assert!(q.submit(&[6], true, false, 0));
        assert_eq!(q.pop().unwrap().bytes, vec![6]);
    }
    #[test]
    fn parameter_sets_can_precede_idr_but_delta_cannot_start_decoder() {
        let mut q = Queue::default();
        assert!(!q.submit(&[1], false, false, 0));
        assert!(q.submit(&[2], false, true, 1));
        assert!(q.report.waiting_for_idr);
        assert!(q.submit(&[3], true, false, 2));
        assert!(!q.report.waiting_for_idr);
        assert!(q.submit(&[4], false, false, 3));
        assert_eq!(q.pop().unwrap().bytes.as_slice(), &[2]);
        let frame = q.pop().unwrap();
        assert!(frame.idr);
        assert_eq!(frame.timestamp_100ns, 2);
        q.clear();
        assert_eq!(q.report.queue_bytes, 0);
    }
    #[test]
    fn stopped_and_oversized_input_are_never_retained() {
        let mut q = Queue {
            stopped: true,
            ..Default::default()
        };
        assert!(!q.submit(b"private-video", true, false, 0));
        assert_eq!(q.report.frames_queued, 0);
        let mut q = Queue::default();
        assert!(!q.submit(
            &vec![0; crate::control::MAX_CHANNEL_MESSAGE + 1],
            true,
            false,
            0
        ));
        assert_eq!(q.report.queue_bytes, 0);
        assert!(!serde_json::to_string(&q.report)
            .unwrap()
            .contains("private-video"));
    }
}
