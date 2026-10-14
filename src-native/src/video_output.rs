//! Bounded compressed-video queue; decoder resources belong to one worker.
use serde::Serialize;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

// Allow bounded headroom while Media Foundation initializes its first device.
pub const MAX_FRAMES: usize = 32;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub codec: Option<crate::video_stream::Codec>,
    pub enabled: bool,
    pub decoder_initialized: bool,
    pub low_latency_request_accepted: bool,
    pub mmcss_registered: bool,
    pub decoder: Option<&'static str>,
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
    pub codec: crate::video_stream::Codec,
    pub bytes: Vec<u8>,
    pub idr: bool,
    pub parameters: bool,
    pub timestamp_100ns: i64,
}

pub struct Queue {
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
        if bytes.len() > crate::attempt::MAX_CHANNEL_MESSAGE {
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

    pub fn pop(&mut self) -> Option<Frame> {
        let frame = self.frames.pop_front()?;
        self.report.queue_bytes -= frame.bytes.len();
        self.report.queue_depth = self.frames.len();
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
            &vec![0; crate::attempt::MAX_CHANNEL_MESSAGE + 1],
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
