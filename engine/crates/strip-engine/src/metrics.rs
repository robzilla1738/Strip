//! AX write timings and frame pacing for the `metrics` query.

use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BundleTiming {
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Metrics {
    writes: BTreeMap<String, Vec<f64>>,
    frame_intervals_ms: Vec<f64>,
    pub last_frame_ms: f64,
    pub frames_per_second: f64,
    pub idle_cpu: f64,
    pub size_writes_during_animation: u64,
    pub display_link_running: bool,
    pub frame_interval_p95_ms: f64,
    pub refresh_interval_ms: f64,
}

impl Metrics {
    pub fn record_write(&mut self, bundle_id: &str, elapsed_ms: f64) {
        // Percentiles only need a recent window; an unbounded history makes
        // every metrics snapshot (taken per animation frame) slower all day.
        const KEEP: usize = 512;
        let samples = self.writes.entry(bundle_id.to_string()).or_default();
        samples.push(elapsed_ms);
        if samples.len() > KEEP {
            let extra = samples.len() - KEEP;
            samples.drain(0..extra);
        }
    }

    pub fn record_frame(&mut self, interval_ms: f64) {
        self.last_frame_ms = interval_ms;
        self.frame_intervals_ms.push(interval_ms);
        if interval_ms > 0.0 {
            self.frames_per_second = 1000.0 / interval_ms;
        }
    }

    pub fn bundle(&self, bundle_id: &str) -> BundleTiming {
        let Some(samples) = self.writes.get(bundle_id) else {
            return BundleTiming::default();
        };
        BundleTiming {
            p50_ms: percentile(samples, 50.0),
            p95_ms: percentile(samples, 95.0),
            max_ms: samples.iter().copied().fold(0.0, f64::max),
        }
    }

    pub fn bundles(&self) -> BTreeMap<String, BundleTiming> {
        self.writes.keys().map(|key| (key.clone(), self.bundle(key))).collect()
    }
}

pub fn percentile(samples: &[f64], percent: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut ordered = samples.to_vec();
    ordered.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let rank = ((percent / 100.0) * (ordered.len() as f64 - 1.0)).round() as usize;
    ordered[rank.min(ordered.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_follow_the_recorded_samples() {
        let mut metrics = Metrics::default();
        for value in [10.0, 20.0, 30.0, 40.0, 100.0] {
            metrics.record_write("com.example.App", value);
        }
        let timing = metrics.bundle("com.example.App");
        assert_eq!(timing.p50_ms, 30.0);
        assert_eq!(timing.max_ms, 100.0);
        assert!(timing.p95_ms >= 40.0);
    }
}
