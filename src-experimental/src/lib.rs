//! Experimental native adapters. Never linked by the default WebRTC client.
pub mod account_socket;
pub mod bud;
#[cfg(all(windows, feature = "gpu-probes"))]
pub mod intel;
pub mod media;
pub mod signaling;
