//! Opt-in BUD media handoff to the existing native decoders.
//! No alternate decoder, renderer, decoded-frame export or GPU interop path.
use anyhow::{ensure, Result};
use parsec_native_bud::{
    bud::{MediaSource, Session},
    media::{self, Rotation, Video},
};
use std::sync::Arc;

/// Both the initial attempt and an automatic relay retry use the same capabilities.
pub(crate) fn session(
    hevc: bool,
    ten_bit: bool,
    raw: bool,
    layout: crate::audio_layout::Layout,
) -> Result<Session> {
    let mut session = Session::external_video_depth(hevc, ten_bit)?;
    session.allow_raw_audio(raw);
    session.set_audio_layout(layout.channels, layout.mask)?;
    crate::event_log::record(
        "INFO",
        "audio_capabilities_declared",
        serde_json::json!({
            "transport":"BUD", "raw_pcm_allowed":raw, "channels":layout.channels, "channel_mask":layout.mask
        }),
    );
    Ok(session)
}

pub struct Media {
    video: crate::video_windows::Pipeline,
    audio: Arc<crate::audio_stream::Pipeline>,
    audio_sender: crate::audio_stream::Sender,
    inspector: crate::video_stream::Inspector,
    waiting_for_keyframe: bool,
    hevc_available: bool,
    ten_bit_available: bool,
}

#[derive(Default)]
pub struct Pumped {
    pub video_messages: usize,
    pub audio_messages: usize,
    pub video_bytes: usize,
    pub audio_bytes: usize,
    pub accepted_video: usize,
    pub accepted_audio: usize,
}

impl Media {
    /// Call only after an explicit BUD selection. Account/UI wiring is separate;
    /// this does not start any connection or change a WebRTC session.
    pub fn start_verified(
        window: Arc<crate::window::Window>,
        hevc_available: bool,
        ten_bit: bool,
        raw_audio: bool,
        surround: bool,
    ) -> Result<(Self, Session)> {
        let layout = if surround {
            crate::audio_windows::device_layout().unwrap_or(crate::audio_layout::Layout::STEREO)
        } else {
            crate::audio_layout::Layout::STEREO
        };
        let session = session(hevc_available, ten_bit, raw_audio, layout)?;
        let audio = Arc::new(crate::audio_stream::Pipeline::start_with_layout(
            window.audio_buffer().map(|(_, max)| max),
            layout,
        ));
        let audio_sender = audio
            .sender()
            .ok_or_else(|| anyhow::anyhow!("native audio sender unavailable"))?;
        let video = crate::video_windows::Pipeline::start(window);
        let mut inspector = crate::video_stream::Inspector::default();
        inspector.enable_hevc(hevc_available);
        Ok((
            Self {
                video,
                audio,
                audio_sender,
                inspector,
                waiting_for_keyframe: true,
                hevc_available,
                ten_bit_available: hevc_available && ten_bit,
            },
            session,
        ))
    }

    fn video_message(&mut self, message: &[u8]) -> Result<bool> {
        let parsed = media::video(message).inspect_err(|_| {
            crate::event_log::record(
                "ERROR",
                "bud_media_rejected",
                serde_json::json!({"stage":"video_framing_or_format"}),
            );
        })?;
        let ten_bit = match &parsed {
            Video::Metadata(metadata) => metadata.ten_bit,
            Video::AccessUnit { header, .. } => header.ten_bit,
        };
        if ten_bit && !self.ten_bit_available {
            crate::event_log::record(
                "ERROR",
                "bud_media_rejected",
                serde_json::json!({"stage":"main10_not_advertised"}),
            );
            anyhow::bail!("BUD host sent Main 10 without an advertised GPU decoder");
        }
        // Depth is validated against the negotiated native capability. The
        // existing pipeline selects MF/P010 from the actual HEVC SPS, without
        // another decoded-frame copy or a new decoder/presentation path.
        match parsed {
            Video::Metadata(metadata) => {
                if metadata.rebuilt {
                    self.waiting_for_keyframe = true;
                    self.inspector = Default::default();
                    self.inspector.enable_hevc(self.hevc_available);
                }
                Ok(false)
            }
            Video::AccessUnit { header, annex_b } => {
                ensure!(
                    matches!(header.rotation, Rotation::None | Rotation::Unknown),
                    "rotated BUD presentation is not connected yet"
                );
                // Existing inspector handles the actual Annex B codec. Native
                // headers from older hosts may label HEVC as H.264.
                let Some(info) = self.inspector.receive(annex_b) else {
                    return Ok(false);
                };
                ensure!(
                    info.codec != crate::video_stream::Codec::Hevc || self.hevc_available,
                    "BUD host sent HEVC without an available native decoder"
                );
                if self.waiting_for_keyframe && !info.idr && !info.parameters {
                    return Ok(false);
                }
                let accepted = self.video.submit(annex_b, info);
                if accepted && info.idr {
                    self.waiting_for_keyframe = false;
                }
                if !accepted {
                    self.waiting_for_keyframe = true;
                }
                Ok(accepted)
            }
        }
    }

    /// Bounded work on the application consumer; network receipt runs in the
    /// SDK thread. No session/global GUI lock is held for decoder processing.
    pub fn pump(&mut self, source: &mut MediaSource) -> Result<Pumped> {
        let mut counts = Pumped::default();
        for _ in 0..8 {
            let Some((accepted, size)) =
                source.with_video(|message| Ok((self.video_message(message)?, message.len())))?
            else {
                break;
            };
            counts.video_messages += 1;
            counts.video_bytes += size;
            counts.accepted_video += usize::from(accepted);
        }
        for _ in 0..8 {
            let Some((accepted, size)) = source.with_audio(|message| {
                Ok((
                    match media::audio_layout(message)? {
                        (media::Audio::Opus(bytes), channels, mask) => {
                            self.audio_sender.submit_layout(
                                bytes,
                                false,
                                crate::audio_layout::Layout::new(channels, mask)?,
                            )
                        }
                        (media::Audio::Pcm(bytes), channels, mask) => {
                            self.audio_sender.submit_layout(
                                bytes,
                                true,
                                crate::audio_layout::Layout::new(channels, mask)?,
                            )
                        }
                    },
                    message.len(),
                ))
            })?
            else {
                break;
            };
            counts.audio_messages += 1;
            counts.audio_bytes += size;
            counts.accepted_audio += usize::from(accepted);
        }
        Ok(counts)
    }

    pub fn recover(&mut self, session: &mut Session) {
        if let Some(generation) = self.video.take_keyframe_request() {
            self.waiting_for_keyframe = true;
            if !session.request_keyframe() {
                self.video.recovery_request_failed(generation);
            }
        }
    }

    /// Preserve the current audio/WASM playback boundary and libopus decoder.
    pub fn poll_audio(
        &self,
        memory: &crate::memory::GuestMemory,
        pointer: u32,
        capacity: usize,
    ) -> Result<usize> {
        self.audio.poll(memory, pointer, capacity)
    }

    pub fn audio_pipeline(&self) -> Arc<crate::audio_stream::Pipeline> {
        self.audio.clone()
    }
    pub fn audio_layout(&self) -> crate::audio_layout::Layout {
        self.audio.allowed_layout()
    }

    pub fn ingress(&self) -> crate::media_ingress::Ingress {
        crate::media_ingress::Ingress {
            video_stream: self.inspector.snapshot(),
            video_output: Some(self.video.snapshot()),
            audio_output: Some(self.audio.snapshot()),
            video_decoder_available: true,
            audio_decoder_available: true,
            ..Default::default()
        }
    }

    pub fn video_snapshot(&self) -> crate::video_output::Snapshot {
        self.video.snapshot()
    }
    pub fn audio_snapshot(&self) -> crate::audio_stream::Snapshot {
        self.audio.snapshot()
    }
}
