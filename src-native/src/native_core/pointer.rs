//! lowlat's cache/forget/unchanged rules with our existing Win32 cursor owner.
use anyhow::{ensure, Result};
use evoparseq_bud::bud::{cursor_message, CursorCache, CursorFlags, CursorShape};
#[derive(Default)]
pub struct Pointer {
    cache: CursorCache,
    delivered: Option<u32>,
}
impl Pointer {
    pub fn receive(&mut self, bytes: &[u8], window: &crate::window::Window) -> Result<()> {
        let pointer = cursor_message(bytes)?;
        if pointer.update.stream != 0 {
            return Ok(());
        }
        if pointer.flags.contains(CursorFlags::FORGET) {
            self.cache.clear();
            self.delivered = None;
        }
        window.host_cursor_event(&serde_json::json!({"type":1,"cursor":{
            "positionX":pointer.update.x,"positionY":pointer.update.y,
            "relative":pointer.update.relative,"hidden":pointer.update.hidden}}));
        window.set_cursor(crate::cursor::Request::Visible(!pointer.update.suppressed));
        let checksum = if !pointer.image.is_empty() {
            ensure!(
                pointer.image.len() <= 1024 * 1024,
                "Host cursor exceeds limit"
            );
            Some(self.cache.insert(
                CursorShape {
                    width: pointer.width,
                    height: pointer.height,
                    hot_x: pointer.update.hot_x,
                    hot_y: pointer.update.hot_y,
                },
                pointer.image,
            ))
        } else if pointer.flags.contains(CursorFlags::CACHED) {
            Some(pointer.checksum)
        } else {
            None
        };
        if let Some(checksum) = checksum.filter(|sum| self.delivered != Some(*sum)) {
            if let Some((shape, png)) = self.cache.get(checksum) {
                match crate::cursor::Frame::native_png(
                    png,
                    i32::from(shape.hot_x),
                    i32::from(shape.hot_y),
                ) {
                    Ok(image) => {
                        window.set_cursor(crate::cursor::Request::Image(Some(image)));
                        self.delivered = Some(checksum);
                    }
                    Err(_) => window.set_cursor(crate::cursor::Request::Image(None)),
                }
            }
        }
        Ok(())
    }
}
