//! Bounded mouse-only capture for this test branch. Written once after shutdown.
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Instant,
};

const CAPACITY: usize = 4096;
#[derive(Serialize)]
struct Record {
    microseconds: u128,
    stage: &'static str,
    values: [i32; 4],
}
struct State {
    started: Instant,
    records: Vec<Record>,
    omitted: u64,
}
static TRACE: OnceLock<Mutex<State>> = OnceLock::new();
pub struct Guard(PathBuf);
pub fn start(path: PathBuf) -> Guard {
    let _ = TRACE.set(Mutex::new(State {
        started: Instant::now(),
        records: Vec::with_capacity(CAPACITY),
        omitted: 0,
    }));
    parsec_native_bud::input_trace::install(record);
    Guard(path)
}
pub fn record(stage: &'static str, values: [i32; 4]) {
    if !matches!(
        stage,
        "host_cursor"
            | "cursor_transport"
            | "cursor_worker_poll"
            | "cursor_control_queued"
            | "cursor_backend_decode"
            | "cursor_callback_wake"
            | "cursor_callback_run"
            | "win_button"
            | "win_capture_changed"
            | "relative_request"
            | "cursor_request"
            | "cursor_selected"
            | "cursor_applied"
            | "capture_applied"
    ) {
        return;
    }
    let Some(trace) = TRACE.get() else { return };
    let mut state = trace.lock().unwrap_or_else(|e| e.into_inner());
    if !matches!(
        stage,
        "cursor_transport"
            | "cursor_worker_poll"
            | "cursor_control_queued"
            | "cursor_backend_decode"
    ) && state
        .records
        .last()
        .is_some_and(|record| record.stage == stage && record.values == values)
    {
        return;
    }
    if state.records.len() == CAPACITY {
        state.omitted += 1;
        return;
    }
    let microseconds = state.started.elapsed().as_micros();
    state.records.push(Record {
        microseconds,
        stage,
        values,
    });
}
impl Drop for Guard {
    fn drop(&mut self) {
        let Some(trace) = TRACE.get() else { return };
        let state = trace.lock().unwrap_or_else(|e| e.into_inner());
        let report = serde_json::json!({
            "schema":4,"commit":"cursor-callback-wakeup-candidate",
            "capacity":CAPACITY,"omitted":state.omitted,"records":state.records,
        });
        if let Ok(bytes) = serde_json::to_vec(&report) {
            let _ = std::fs::write(&self.0, bytes);
        }
    }
}
