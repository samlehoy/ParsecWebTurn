//! Experimental native adapters. Never linked by the default WebRTC client.
pub mod bud;
#[cfg(all(windows, feature = "gpu-probes"))]
pub mod gpu;
#[cfg(all(windows, feature = "gpu-probes"))]
pub mod intel;
#[cfg(all(windows, feature = "gpu-probes"))]
pub mod intel_hevc;
pub mod media;
pub mod signaling;
