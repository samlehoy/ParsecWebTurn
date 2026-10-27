//! Bounded recent averages from existing telemetry totals, not new frame work.
use crate::stats::Sample;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
const WINDOW: Duration = Duration::from_secs(2);
const CAPACITY: usize = 16;
struct Observation {
    at: Instant,
    points: [(u64, u128); 8],
}
pub struct Recent {
    generation: u64,
    observations: VecDeque<Observation>,
}
impl Default for Recent {
    fn default() -> Self {
        Self {
            generation: 0,
            observations: VecDeque::with_capacity(CAPACITY),
        }
    }
}
impl Recent {
    pub fn update(&mut self, sample: &Sample, connected: bool) -> [Option<f64>; 8] {
        let Some(video) = sample
            .video
            .as_ref()
            .filter(|v| connected && v.decoder_initialized && !v.recovering && !v.worker_finished)
        else {
            self.observations.clear();
            return [None; 8];
        };
        let points = video
            .local_timing
            .stages()
            .map(|(_, t)| (t.count, t.total_ns));
        if self.generation != sample.generation
            || self.observations.back().is_some_and(|old| {
                sample.at < old.at
                    || sample.at.duration_since(old.at) > WINDOW
                    || points
                        .iter()
                        .zip(old.points)
                        .any(|(&(count, total), (c, t))| count < c || total < t)
            })
        {
            self.observations.clear();
        }
        self.generation = sample.generation;
        while self
            .observations
            .front()
            .is_some_and(|old| sample.at.saturating_duration_since(old.at) > WINDOW)
        {
            self.observations.pop_front();
        }
        if self
            .observations
            .back()
            .is_none_or(|old| old.at != sample.at)
        {
            if self.observations.len() == CAPACITY {
                self.observations.pop_front();
            }
            self.observations.push_back(Observation {
                at: sample.at,
                points,
            });
        }
        let Some(old) = self.observations.front() else {
            return [None; 8];
        };
        std::array::from_fn(|i| {
            let count = points[i].0.checked_sub(old.points[i].0)?;
            let total = points[i].1.checked_sub(old.points[i].1)?;
            (count > 0).then(|| total as f64 / count as f64 / 1e6)
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recent_average_is_weighted_expires_and_resets_with_counter_or_session() {
        let mut recent = Recent::default();
        let mut sample = Sample {
            generation: 1,
            video: Some(crate::video_output::Snapshot {
                decoder_initialized: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let add = |s: &mut Sample, n: usize, ms| {
            s.at += Duration::from_millis(250);
            for _ in 0..n {
                s.video
                    .as_mut()
                    .unwrap()
                    .local_timing
                    .decoder_service
                    .record(Duration::from_millis(ms));
            }
        };
        assert_eq!(recent.update(&sample, true)[2], None);
        add(&mut sample, 10, 1);
        assert_eq!(recent.update(&sample, true)[2], Some(1.0));
        add(&mut sample, 1, 12);
        assert_eq!(recent.update(&sample, true)[2], Some(2.0));
        // Re-reading the same telemetry cannot add samples or change weighting.
        assert_eq!(recent.update(&sample, true)[2], Some(2.0));
        sample.at += Duration::from_secs(3);
        assert_eq!(recent.update(&sample, true)[2], None);
        add(&mut sample, 1, 3);
        assert_eq!(recent.update(&sample, true)[2], Some(3.0));
        sample.video.as_mut().unwrap().local_timing = Default::default();
        sample.at += Duration::from_millis(250);
        assert_eq!(recent.update(&sample, true)[2], None);
        add(&mut sample, 1, 4);
        assert_eq!(recent.update(&sample, true)[2], Some(4.0));
        sample.generation += 1;
        assert_eq!(recent.update(&sample, true)[2], None);
        assert_eq!(recent.update(&sample, false)[2], None);
        assert!(recent.observations.is_empty());
    }
    #[test]
    fn history_is_bounded_and_silence_is_unknown_not_an_old_measurement() {
        let mut recent = Recent::default();
        let mut sample = Sample {
            video: Some(crate::video_output::Snapshot {
                decoder_initialized: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        for _ in 0..100 {
            sample.at += Duration::from_millis(10);
            sample
                .video
                .as_mut()
                .unwrap()
                .local_timing
                .present_call
                .record(Duration::from_millis(1));
            recent.update(&sample, true);
        }
        assert_eq!(recent.observations.len(), CAPACITY);
        for _ in 0..10 {
            sample.at += Duration::from_millis(250);
            recent.update(&sample, true);
        }
        assert_eq!(recent.update(&sample, true)[6], None);
        sample.video.as_mut().unwrap().recovering = true;
        assert_eq!(recent.update(&sample, true), [None; 8]);
    }
}
