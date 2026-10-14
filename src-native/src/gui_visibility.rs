//! Coalesced native GUI coverage transitions, consumed only by its render worker.
//! Even a fast open/Back must be observed as hidden, then visible, by the core.
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Mutex,
};

struct Pending {
    covered: bool,
    hide_pending: bool,
    applied_visible: bool,
}
pub struct State {
    app: AtomicU32,
    pending: Mutex<Pending>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            app: AtomicU32::new(0),
            pending: Mutex::new(Pending {
                covered: false,
                hide_pending: false,
                applied_visible: true,
            }),
        }
    }
}
impl State {
    pub fn publish_app(&self, app: u32) {
        self.app.store(app, Ordering::Release);
    }
    pub fn cover(&self, covered: bool) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.covered = covered;
        pending.hide_pending |= covered;
    }
    pub fn next(&self) -> Option<(u32, bool)> {
        let app = self.app.load(Ordering::Acquire);
        if app == 0 {
            return None;
        }
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let visible = if pending.hide_pending {
            pending.hide_pending = false;
            false
        } else {
            !pending.covered
        };
        if pending.applied_visible == visible {
            return None;
        }
        pending.applied_visible = visible;
        Some((app, visible))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fast_back_preserves_both_visibility_phases_without_an_unbounded_queue() {
        let state = State::default();
        for _ in 0..1000 {
            state.cover(true);
            state.cover(false);
        }
        assert_eq!(state.next(), None); // Do not consume before core initialization.
        state.publish_app(42);
        assert_eq!(state.next(), Some((42, false)));
        assert_eq!(state.next(), Some((42, true)));
        assert_eq!(state.next(), None);
        state.cover(true);
        assert_eq!(state.next(), Some((42, false)));
        assert_eq!(state.next(), None);
        state.cover(false);
        assert_eq!(state.next(), Some((42, true)));
    }
}
