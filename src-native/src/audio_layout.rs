//! Validated Windows speaker layouts used by the native BUD audio bridge.
use anyhow::{bail, Result};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Layout {
    pub channels: u8,
    pub mask: u32,
}
impl Layout {
    pub const STEREO: Self = Self {
        channels: 2,
        mask: 3,
    };

    pub fn new(channels: u8, mask: u32) -> Result<Self> {
        match (channels, mask) {
            (2, 0 | 3) => Ok(Self::STEREO),
            (6, 0x3f | 0x60f) | (8, 0x63f) => Ok(Self { channels, mask }),
            _ => bail!("unsupported native speaker layout"),
        }
    }

    pub fn samples(self, frames: usize) -> usize {
        frames * usize::from(self.channels)
    }

    /// Parsec's multistream contract, in Windows speaker order. These are not
    /// Vorbis output channel positions; the mapping describes the wire streams.
    pub fn opus(self) -> (i32, i32, [u8; 8]) {
        match self.channels {
            6 => (4, 2, [0, 1, 4, 5, 2, 3, 0, 0]),
            8 => (5, 3, [0, 1, 6, 7, 2, 3, 4, 5]),
            _ => (1, 1, [0, 1, 0, 0, 0, 0, 0, 0]),
        }
    }

    /// The unchanged WASM mixer takes stereo. Give it a valid reference mix
    /// while the native queue retains the original channels for presentation.
    /// Its queue/reset decisions still own mute and playback; no audio is sent
    /// to the device until that exact polled block is queued by the core.
    pub fn stereo_mix(self, pcm: &[i16]) -> Vec<i16> {
        let mut output = Vec::with_capacity(pcm.len() / usize::from(self.channels) * 2);
        for frame in pcm.chunks_exact(usize::from(self.channels)) {
            let mut left = f32::from(frame[0]);
            let mut right = f32::from(frame[1]);
            if self.channels > 2 {
                let center = f32::from(frame[2]) * std::f32::consts::FRAC_1_SQRT_2;
                left += center;
                right += center;
                // LFE is omitted from the reference stereo fold-down.
                for pair in frame[4..].as_chunks::<2>().0 {
                    left += f32::from(pair[0]) * std::f32::consts::FRAC_1_SQRT_2;
                    right += f32::from(pair[1]) * std::f32::consts::FRAC_1_SQRT_2;
                }
            }
            output.extend([left as i16, right as i16]);
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn speaker_masks_and_wire_mapping_are_validated() {
        for (channels, mask) in [(6, 0x3f), (6, 0x60f), (8, 0x63f)] {
            let layout = Layout::new(channels, mask).unwrap();
            let (streams, coupled, mapping) = layout.opus();
            assert_eq!(streams + coupled, i32::from(channels));
            let mut indices = mapping[..usize::from(channels)].to_vec();
            indices.sort();
            assert_eq!(indices, (0..channels).collect::<Vec<_>>());
        }
        assert!(Layout::new(6, 3).is_err());
        assert!(Layout::new(8, 0x3f).is_err());
        assert_eq!(Layout::new(2, 0).unwrap(), Layout::STEREO);
    }
}
