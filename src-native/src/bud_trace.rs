//! Opt-in, bounded BUD connection timing only. Never record wire payloads,
//! credentials, peer addresses, account identifiers or arbitrary error strings.
use std::{fs::File, io::Write, time::Instant};

pub struct Trace {
    file: Option<File>,
    began: Instant,
    stage: &'static str,
    records: usize,
}

impl Trace {
    pub fn start() -> Self {
        let file = (std::env::var_os("PARSEC_BUD_DIAGNOSTICS").as_deref()
            == Some(std::ffi::OsStr::new("1")))
        .then(|| {
            let exe = std::env::current_exe().ok()?;
            File::create(exe.parent()?.join("bud-diagnostics.jsonl")).ok()
        })
        .flatten();
        let mut trace = Self {
            file,
            began: Instant::now(),
            stage: "start",
            records: 0,
        };
        trace.mark("start");
        trace
    }
    pub fn mark(&mut self, stage: &'static str) {
        self.stage = stage;
        self.record(stage, &[]);
    }
    pub fn record(&mut self, event: &'static str, counters: &[(&'static str, u64)]) {
        let Some(file) = &mut self.file else { return };
        if self.records >= 1024 {
            return;
        }
        let entry = serde_json::json!({
            "ms": self.began.elapsed().as_millis(),
            "event": event, "stage": self.stage,
            "build": env!("CARGO_PKG_VERSION"),
            "counters": counters.iter().map(|(key, value)| ((*key).to_owned(), serde_json::json!(value))).collect::<serde_json::Map<_, _>>()
        });
        if writeln!(file, "{entry}").is_err() {
            self.file = None;
        }
        self.records += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_is_bounded_and_contains_only_timing_categories_and_counters() {
        let path = std::env::temp_dir().join(format!(
            "pwt-bud-trace-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut trace = Trace {
            file: Some(File::create(&path).unwrap()),
            began: Instant::now(),
            stage: "relay_retry_started",
            records: 0,
        };
        for _ in 0..1100 {
            trace.record("transport_progress", &[("udp_received", 42)]);
        }
        drop(trace);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(text.lines().count(), 1024);
        for line in text.lines() {
            let entry: serde_json::Value = serde_json::from_str(line).unwrap();
            let keys: Vec<_> = entry
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(keys, ["build", "counters", "event", "ms", "stage"]);
            assert_eq!(entry["counters"], serde_json::json!({"udp_received": 42}));
        }
    }
}
