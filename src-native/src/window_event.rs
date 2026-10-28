#[derive(Clone, Debug)]
pub enum Event {
    Size(i32, i32),
    Focus(bool),
    Clipboard,
    // Local scheduling hint only; never translated into remote mouse input.
    CursorReady,
    Motion(i32, i32),
    RelativeMotion(i32, i32),
    RelativeMode(bool),
    Button(bool, i32, i32, i32),
    Text(u32),
    Key(bool, &'static str, i32),
    Scroll(i32, i32),
    Fullscreen(bool),
    #[cfg(feature = "bud")]
    HidInputRelease,
}
