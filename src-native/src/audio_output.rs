//! Local output controls; independent of transport and platform audio APIs.
use std::sync::atomic::{AtomicU32, Ordering};

const MUTED: u32 = 1 << 8;

pub struct Output(AtomicU32);

impl Default for Output {
    fn default() -> Self {
        Self(AtomicU32::new(100))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    pub percent: u32,
    pub muted: bool,
}

impl Level {
    pub fn gain(self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.percent as f32 / 100.0
        }
    }
}

impl Output {
    pub fn level(&self) -> Level {
        let value = self.0.load(Ordering::Relaxed);
        Level {
            percent: value & 0xff,
            muted: value & MUTED != 0,
        }
    }

    pub fn set_percent(&self, percent: u32) {
        let percent = percent.min(100);
        let mut value = self.0.load(Ordering::Relaxed);
        loop {
            match self.0.compare_exchange_weak(
                value,
                (value & MUTED) | percent,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(current) => value = current,
            }
        }
    }

    pub fn set_muted(&self, muted: bool) {
        if muted {
            self.0.fetch_or(MUTED, Ordering::Relaxed);
        } else {
            self.0.fetch_and(!MUTED, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mute_preserves_volume_and_volume_preserves_mute() {
        let output = Output::default();
        assert_eq!(output.level().gain(), 1.0);
        output.set_percent(40);
        output.set_muted(true);
        assert_eq!(output.level().gain(), 0.0);
        output.set_percent(70);
        assert!(output.level().muted);
        output.set_muted(false);
        assert_eq!(output.level().gain(), 0.7);
        output.set_percent(u32::MAX);
        assert_eq!(output.level().percent, 100);
        output.set_percent(0);
        assert_eq!(output.level().gain(), 0.0);
    }
}
