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
