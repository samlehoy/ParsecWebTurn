//! Documented Windows screen power request, confined to the window UI thread.
use serde::Serialize;
use windows_sys::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED,
};

#[derive(Clone, Default, Serialize)]
pub struct State {
    pub requested: bool,
    pub applied: bool,
    pub failed_requests: u64,
}
impl State {
    pub fn update(&mut self, requested: bool, visible: bool) {
        self.update_with(requested, visible, |enable| unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | if enable { ES_DISPLAY_REQUIRED } else { 0 })
                != 0
        });
    }
    fn update_with(&mut self, requested: bool, visible: bool, mut apply: impl FnMut(bool) -> bool) {
        self.requested = requested;
        let desired = requested && visible;
        if desired == self.applied {
            return;
        }
        if apply(desired) {
            self.applied = desired;
        } else {
            self.failed_requests = self.failed_requests.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_request_and_release_share_one_thread() {
        std::thread::spawn(|| {
            let mut state = State::default();
            state.update(true, true);
            assert!(state.applied || state.failed_requests == 1);
            state.update(false, false);
            // A platform rejection is recorded without terminating playback.
            assert!(!state.requested);
            unsafe {
                SetThreadExecutionState(ES_CONTINUOUS);
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn visibility_and_release_are_idempotent() {
        let mut state = State::default();
        let mut calls = Vec::new();
        for (request, visible) in [
            (true, true),
            (true, true),
            (true, false),
            (true, true),
            (false, true),
            (false, true),
        ] {
            state.update_with(request, visible, |enable| {
                calls.push(enable);
                true
            });
        }
        assert_eq!(calls, [true, false, true, false]);
        assert!(!state.applied && !state.requested);
    }
    #[test]
    fn rejection_preserves_actual_state_and_retries_release() {
        let mut state = State::default();
        state.update_with(true, true, |_| false);
        assert!(!state.applied);
        state.update_with(true, true, |_| true);
        state.update_with(false, true, |_| false);
        assert!(state.applied && !state.requested);
        state.update_with(false, true, |_| true);
        assert!(!state.applied);
        assert_eq!(state.failed_requests, 2);
    }
}
