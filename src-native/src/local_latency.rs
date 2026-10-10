//! Temporary dev-only input timing display, bounded and payload-free. No receive-path timing.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const LIMIT: usize = 1024;
const AGE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Default)]
pub struct Summary {
    pub count: usize,
    pub mean_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

#[derive(Clone, Default)]
pub struct Window {
    samples: VecDeque<(Instant, Duration)>,
}

impl Window {
    fn prune(&mut self, now: Instant) {
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > AGE)
        {
            self.samples.pop_front();
        }
    }

    pub fn record(&mut self, started: Instant, ended: Instant) {
        self.prune(ended);
        if self.samples.len() == LIMIT {
            self.samples.pop_front();
        }
        self.samples
            .push_back((ended, ended.saturating_duration_since(started)));
    }

    pub fn snapshot(&mut self, now: Instant) -> Summary {
        self.prune(now);
        let mut values: Vec<_> = self
            .samples
            .iter()
            .map(|(_, d)| d.as_secs_f64() * 1000.0)
            .collect();
        if values.is_empty() {
            return Summary::default();
        }
        values.sort_unstable_by(f64::total_cmp);
        let count = values.len();
        Summary {
            count,
            mean_ms: values.iter().sum::<f64>() / count as f64,
            p95_ms: values[(count * 95).div_ceil(100) - 1],
            max_ms: values[count - 1],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_recent_samples_and_nearest_rank_percentile() {
        let now = Instant::now();
        let mut window = Window::default();
        assert_eq!(window.snapshot(now).count, 0);
        for ms in 1..=100 {
            window.record(now - Duration::from_millis(ms), now);
        }
        let summary = window.snapshot(now);
        assert_eq!(summary.count, 100);
        assert!((summary.mean_ms - 50.5).abs() < 1e-9);
        assert_eq!(summary.p95_ms, 95.0);
        assert_eq!(summary.max_ms, 100.0);
        assert_eq!(
            window.snapshot(now + AGE + Duration::from_nanos(1)).count,
            0
        );
        for _ in 0..LIMIT + 10 {
            window.record(now, now);
        }
        assert_eq!(window.snapshot(now).count, LIMIT);
    }
}
