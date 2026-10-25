#![cfg_attr(windows, windows_subsystem = "windows")]
#![warn(unused_crate_dependencies)]
// Integration-only TLS fixtures are compiled as separate test crates.
#[cfg(test)]
use {rcgen as _, rustls as _, tokio_rustls as _};

const APP_TITLE: &str = "EvoParseq";
mod attempt;
mod audio_layout;
mod audio_stream;
#[cfg(windows)]
mod audio_windows;
#[cfg(windows)]
mod branding;
#[cfg(all(windows, feature = "bud"))]
mod bud_attempt;
#[cfg(all(windows, feature = "bud"))]
mod bud_media;
mod connection_settings;
#[cfg(windows)]
mod connection_settings_ui;
mod control;
#[cfg(test)]
mod crypto_policy_tests;
#[cfg(windows)]
mod cursor;
#[cfg(test)]
mod data_only_policy_tests;
mod event_log;
mod event_queue;
#[cfg(windows)]
mod frame_timing;
#[cfg(windows)]
mod fullscreen;
#[cfg(windows)]
mod gpu_windows;
#[cfg(windows)]
mod gui_visibility;
#[cfg(all(windows, feature = "bud"))]
mod hid_input;
mod input;
mod input_state;
#[cfg(windows)]
mod keyboard_grab;
mod lifecycle;
mod media_ingress;
#[cfg(windows)]
mod media_scheduling;
#[cfg(windows)]
mod native_core;
mod network_audit;
mod network_policy;
#[cfg(windows)]
mod performance;
mod platform;
#[cfg(windows)]
mod platform_windows;
mod secure_storage;
mod signaling;
#[cfg(windows)]
mod stats;
#[cfg(windows)]
mod stats_ui;
mod transport;
mod video_color;
mod video_output;
mod video_stream;
#[cfg(windows)]
mod video_windows;
mod viewport;
#[cfg(windows)]
mod wake_lock;
#[cfg(windows)]
mod window;
fn main() {
    #[cfg(windows)]
    native_core::main();
    #[cfg(not(windows))]
    eprintln!("EvoParseq currently requires Windows.");
}
