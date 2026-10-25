//! Pinned Parsec channels: 1 video, 2 audio. No decoder or buffered sink.
use serde::Serialize;
#[derive(Clone, Default, Serialize)]
pub struct Ingress {
    pub video_stream: crate::video_stream::Snapshot,
    pub audio_output: Option<crate::audio_stream::Snapshot>,
    pub video_output: Option<crate::video_output::Snapshot>,
    pub video_packets_received: u64,
    pub video_bytes_received: u64,
    pub audio_packets_received: u64,
    pub audio_bytes_received: u64,
    pub packets_discarded_decoder_unavailable: u64,
    pub video_decoder_available: bool,
    pub audio_decoder_available: bool,
}
impl Ingress {
    pub fn unavailable(&mut self, channel: u16, size: usize) {
        self.record(channel, size, true);
    }
    pub fn record(&mut self, channel: u16, size: usize, discarded: bool) {
        let (packets, bytes) = match channel {
            1 => (
                &mut self.video_packets_received,
                &mut self.video_bytes_received,
            ),
            2 => (
                &mut self.audio_packets_received,
                &mut self.audio_bytes_received,
            ),
            _ => return,
        };
        *packets = packets.saturating_add(1);
        *bytes = bytes.saturating_add(size as u64);
        if discarded {
            self.packets_discarded_decoder_unavailable =
                self.packets_discarded_decoder_unavailable.saturating_add(1);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounting_saturates_and_never_allocates_a_media_queue() {
        let mut ingress = Ingress {
            video_packets_received: u64::MAX,
            video_bytes_received: u64::MAX,
            ..Default::default()
        };
        ingress.unavailable(1, 100);
        ingress.unavailable(99, 100);
        assert_eq!(ingress.video_packets_received, u64::MAX);
        assert_eq!(ingress.video_bytes_received, u64::MAX);
        assert_eq!(ingress.packets_discarded_decoder_unavailable, 1);
    }
}
