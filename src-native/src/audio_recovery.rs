//! Portable audio-output recovery policy; endpoint APIs stay in the backend.
use std::time::Duration;

#[derive(Default)]
pub struct Recovery {
    recovering: bool,
    failed_attempts: u8,
}
impl Recovery {
    /// True once per outage, for transition logging rather than retry spam.
    pub fn begin(&mut self) -> bool {
        let first = !self.recovering;
        self.recovering = true;
        first
    }
    pub fn failed(&mut self) -> Duration {
        let delay = (100u64 << self.failed_attempts.min(5)).min(2_000);
        self.failed_attempts = self.failed_attempts.saturating_add(1);
        Duration::from_millis(delay)
    }
    /// A new default endpoint can be tried immediately, without old backoff.
    pub fn endpoint_changed(&mut self) {
        self.failed_attempts = 0;
    }
    pub fn restored(&mut self) -> bool {
        let recovered = self.recovering;
        *self = Self::default();
        recovered
    }
    pub fn recovering(&self) -> bool {
        self.recovering
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_backoff_is_bounded_and_transitions_do_not_repeat() {
        let mut recovery = Recovery::default();
        assert!(!recovery.restored());
        assert!(recovery.begin());
        assert!(!recovery.begin());
        let delays: Vec<_> = (0..100).map(|_| recovery.failed().as_millis()).collect();
        assert_eq!(&delays[..6], &[100, 200, 400, 800, 1600, 2000]);
        assert!(delays[6..].iter().all(|v| *v == 2000));
        recovery.endpoint_changed();
        assert_eq!(recovery.failed(), Duration::from_millis(100));
        assert!(recovery.recovering());
        assert!(recovery.restored());
        assert!(!recovery.recovering());
        assert_eq!(recovery.failed(), Duration::from_millis(100));
    }
}
