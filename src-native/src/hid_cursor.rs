//! Test-build-only cursor policy. Host position controls display; hardware Raw
//! Input controls motion. No video composition, framebuffer readback or copying.
use std::sync::Arc;

pub struct Lifetime {
    window: Arc<crate::window::Window>,
    generation: u64,
}
impl Lifetime {
    pub fn new(window: Arc<crate::window::Window>, generation: u64) -> Self {
        Self { window, generation }
    }
}
impl Drop for Lifetime {
    fn drop(&mut self) {
        self.window.set_usb_cursor_active(self.generation, false);
    }
}

/// The host metadata's position is meaningful for a visible desktop cursor.
/// Relative/hidden host modes retain the original hidden native capture.
pub fn visible_position(cursor: Option<(i32, i32, bool)>) -> Option<(i32, i32)> {
    cursor.and_then(|(x, y, relative)| (!relative).then_some((x, y)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hid_cursor_never_invents_position_or_displays_a_hidden_host_cursor() {
        assert_eq!(visible_position(None), None);
        assert_eq!(visible_position(Some((100, 200, true))), None);
        assert_eq!(visible_position(Some((100, 200, false))), Some((100, 200)));
    }
}
