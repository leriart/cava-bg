// Lightweight performance monitor. Tracks a bounded sliding window of frame
// times so we can log min/avg/max/FPS periodically without allocations in
// the hot path.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub struct PerfMonitor {
    frame_times: VecDeque<Duration>,
    max_samples: usize,
    last_log: Instant,
    log_interval: Duration,
    enabled: bool,
}

impl PerfMonitor {
    pub fn new(max_samples: usize, log_interval_secs: u64, enabled: bool) -> Self {
        Self {
            frame_times: VecDeque::with_capacity(max_samples.max(8)),
            max_samples: max_samples.max(8),
            last_log: Instant::now(),
            log_interval: Duration::from_secs(log_interval_secs.max(1)),
            enabled,
        }
    }

    pub fn reconfigure(&mut self, max_samples: usize, log_interval_secs: u64, enabled: bool) {
        let max_samples = max_samples.max(8);
        self.max_samples = max_samples;
        self.log_interval = Duration::from_secs(log_interval_secs.max(1));
        self.enabled = enabled;
        self.last_log = Instant::now();
        while self.frame_times.len() > self.max_samples {
            self.frame_times.pop_front();
        }
    }

    #[inline(always)]
    pub fn record(&mut self, duration: Duration) {
        if !self.enabled {
            return;
        }
        if self.frame_times.len() >= self.max_samples {
            self.frame_times.pop_front();
        }
        self.frame_times.push_back(duration);
    }

    pub fn maybe_log(&mut self) {
        if !self.enabled || self.frame_times.is_empty() {
            return;
        }
        if self.last_log.elapsed() < self.log_interval {
            return;
        }
        self.last_log = Instant::now();
        let (min, max, avg) = self.stats();
        let fps = if avg.as_secs_f32() > 0.0 {
            1.0 / avg.as_secs_f32()
        } else {
            0.0
        };
        log::info!(
            "perf: fps={:.1} avg={:.2}ms min={:.2}ms max={:.2}ms samples={}",
            fps,
            avg.as_secs_f32() * 1000.0,
            min.as_secs_f32() * 1000.0,
            max.as_secs_f32() * 1000.0,
            self.frame_times.len()
        );
    }

    pub fn avg_frame_time_ms(&self) -> Option<f32> {
        if self.frame_times.is_empty() {
            return None;
        }
        let (_, _, avg) = self.stats();
        Some(avg.as_secs_f32() * 1000.0)
    }

    fn stats(&self) -> (Duration, Duration, Duration) {
        let mut min = Duration::from_secs(9999);
        let mut max = Duration::ZERO;
        let mut sum = Duration::ZERO;
        for d in &self.frame_times {
            if *d < min {
                min = *d;
            }
            if *d > max {
                max = *d;
            }
            sum += *d;
        }
        let n = self.frame_times.len() as u32;
        let avg = if n > 0 { sum / n } else { Duration::ZERO };
        (min, max, avg)
    }
}
