//! Opt-in BUD media handoff to the existing native decoders.
//! No alternate decoder, renderer, decoded-frame export or GPU interop path.
use anyhow::{ensure, Result};
use parsec_native_experimental::{
    bud::Session,
    media::{self, Rotation, Video},
};
use std::sync::Arc;

pub struct Media {
    video: crate::video_windows::Pipeline,
    audio: Arc<crate::audio_stream::Pipeline>,
    audio_sender: crate::audio_stream::Sender,
    inspector: crate::video_stream::Inspector,
    waiting_for_keyframe: bool,
    hevc_available: bool,
}

#[derive(Default)]
pub struct Pumped {
    pub video_messages: usize,
    pub audio_messages: usize,
    pub accepted_video: usize,
    pub accepted_audio: usize,
}

impl Media {
    /// Call only after an explicit BUD selection. Account/UI wiring is separate;
    /// this does not start any connection or change a WebRTC session.
    pub fn start(window: Arc<crate::window::Window>, prefer_hevc: bool) -> Result<(Self, Session)> {
        let hevc_available =
            prefer_hevc && crate::video_windows::hevc_available(window.video_gpu().as_ref());
        let session = Session::external_video(hevc_available)?;
        let audio = Arc::new(crate::audio_stream::Pipeline::start());
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
            },
            session,
        ))
    }

    fn video_message(&mut self, message: &[u8]) -> Result<bool> {
        match media::video(message)? {
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
    pub fn pump(&mut self, session: &mut Session) -> Result<Pumped> {
        let mut counts = Pumped::default();
        for _ in 0..8 {
            let Some(accepted) =
                session.with_video(|message, _arrived_us| self.video_message(message))?
            else {
                break;
            };
            counts.video_messages += 1;
            counts.accepted_video += usize::from(accepted);
        }
        for _ in 0..8 {
            let Some(accepted) = session
                .with_audio(|message| Ok(self.audio_sender.submit(media::opus(message)?)))?
            else {
                break;
            };
            counts.audio_messages += 1;
            counts.accepted_audio += usize::from(accepted);
        }
        if let Some(generation) = self.video.take_keyframe_request() {
            self.waiting_for_keyframe = true;
            if !session.request_keyframe() {
                self.video.recovery_request_failed(generation);
            }
        }
        Ok(counts)
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
