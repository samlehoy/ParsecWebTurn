//! Experimental native adapters. Never linked by the default WebRTC client.
pub mod bud;
#[cfg(windows)]
pub mod intel;
pub mod media;
pub mod signaling;
