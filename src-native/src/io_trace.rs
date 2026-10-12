//! Explicit, bounded troubleshooting capture. Never records input or media payloads.
#[cfg(not(feature = "diagnostics"))]
use anyhow::Result;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};
#[cfg(not(feature = "diagnostics"))]
use std::{
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
};

static ENABLED: AtomicBool = AtomicBool::new(false);
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}
#[cfg(not(feature = "diagnostics"))]
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

#[derive(Default, Serialize)]
pub struct Input {
    pub cursor_events: u64,
    pub relative_events: u64,
    pub hidden_events: u64,
    pub last_relative: bool,
    pub last_hidden: bool,
    pub raw_motion_events: u64,
    pub raw_absolute_events: u64,
    pub clip_error: Option<u32>,
    pub raw_registration_error: Option<u32>,
    pub core_reads: BTreeMap<&'static str, CoreRead>,
}
#[derive(Serialize)]
pub struct CoreRead {
    pub calls: u64,
    pub failures: u64,
    pub app_present: bool,
    pub state_valid: bool,
    pub applied_mode: crate::core_input::Mode,
    pub actual_mode: Option<crate::core_input::Mode>,
}

#[cfg(not(feature = "diagnostics"))]
pub fn start(
    path: &Path,
    window: Arc<crate::window::Window>,
    backend: Arc<Mutex<crate::backend::Backend>>,
) -> Result<std::thread::JoinHandle<()>> {
    let mut file = std::fs::File::create(path)?;
    let header = serde_json::json!({
        "schema": 3, "scope": "opt-in-input-audio-counters",
        "app": env!("CARGO_PKG_VERSION"), "core": crate::PARSEC_CORE_VERSION,
        "build": option_env!("GITHUB_SHA").unwrap_or("local"),
        "interval_ms": 1000, "sample_limit": 600,
        "payloads_recorded": false,
    });
    writeln!(file, "{header}")?;
    Ok(std::thread::Builder::new().name("parsec-io-trace".into()).spawn(move || {
        let started = std::time::Instant::now();
        for _ in 0..600 {
            if window.closing.load(Ordering::Acquire) { break; }
            let audio = backend.try_lock().ok().map(|backend| {
                serde_json::json!({
                    "connected": backend.status == Some(0),
                    "decoder": backend.native_attempt.as_ref().and_then(|a| a.audio_snapshot()),
                    "udp_receive_buffers": backend.native_attempt.as_ref().map(|a| a.udp_buffer_snapshot()),
                })
            });
            let record = serde_json::json!({
                "elapsed_ms": started.elapsed().as_millis(),
                "input": window.input_trace_snapshot(),
                "transport_audio": audio,
                "playback": window.audio_registry.lock().unwrap_or_else(|e| e.into_inner()).snapshots(),
            });
            if writeln!(file, "{record}").and_then(|_| file.flush()).is_err() { break; }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    })?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn input_trace_contains_no_positions_keys_or_cursor_images() {
        let value = serde_json::to_value(super::Input::default()).unwrap();
        let keys: Vec<_> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "clip_error",
                "core_reads",
                "cursor_events",
                "hidden_events",
                "last_hidden",
                "last_relative",
                "raw_absolute_events",
                "raw_motion_events",
                "raw_registration_error",
                "relative_events"
            ]
        );
    }
}
