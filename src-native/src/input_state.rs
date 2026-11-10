//! Local immersive-input state shared by Win32 capture and session controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Mode {
    pub mouse: bool,
    pub keyboard: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct State {
    pub mode: Mode,
    pub detached: bool,
    pub warp_disabled: bool,
}
