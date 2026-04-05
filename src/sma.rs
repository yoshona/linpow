//! Time-weighted Simple Moving Average (SMA).
//!
//! Unlike a plain arithmetic mean, each sample is weighted by the wall-clock
//! duration it was "active" (i.e. the gap until the next sample). This avoids
//! skew when samples arrive at irregular intervals.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub struct TimeSma {
    buf: VecDeque<(Instant, f32)>,
    /// Window in seconds. 0 means "no averaging — return the last raw sample".
    window_secs: f64,
}

impl TimeSma {
    pub fn new(window_secs: f64) -> Self {
        Self {
            buf: VecDeque::new(),
            window_secs,
        }
    }

    pub fn push(&mut self, value: f32) {
        self.buf.push_back((Instant::now(), value));
        self.trim();
    }

    pub fn set_window(&mut self, secs: f64) {
        self.window_secs = secs;
        self.trim();
    }

    /// Get the current averaged value.
    ///
    /// When `window_secs == 0` or the buffer has ≤ 1 sample, returns the
    /// latest raw sample (no smoothing).
    pub fn get(&self) -> f32 {
        if self.buf.is_empty() {
            return 0.0;
        }
        if self.window_secs == 0.0 || self.buf.len() <= 1 {
            return self.buf.back().map(|x| x.1).unwrap_or(0.0);
        }
        let now = Instant::now();
        let cutoff = now - Duration::from_secs_f64(self.window_secs);
        let items: Vec<_> = self.buf.iter().filter(|(t, _)| *t >= cutoff).collect();
        if items.is_empty() {
            return self.buf.back().map(|x| x.1).unwrap_or(0.0);
        }
        let mut weighted_sum = 0.0f64;
        let mut total_duration = 0.0f64;
        for i in 0..items.len() {
            // Each sample's weight = time until the next sample (or "now" for the latest).
            let dt = if i + 1 < items.len() {
                items[i + 1].0.duration_since(items[i].0).as_secs_f64()
            } else {
                now.duration_since(items[i].0).as_secs_f64()
            };
            weighted_sum += items[i].1 as f64 * dt;
            total_duration += dt;
        }
        if total_duration > 0.0 {
            (weighted_sum / total_duration) as f32
        } else {
            self.buf.back().map(|x| x.1).unwrap_or(0.0)
        }
    }

    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Evict samples older than the window to bound memory usage.
    fn trim(&mut self) {
        if self.window_secs == 0.0 {
            // Keep only the latest sample (no history needed).
            while self.buf.len() > 1 {
                self.buf.pop_front();
            }
            return;
        }
        let cutoff = Instant::now() - Duration::from_secs_f64(self.window_secs + 1.0);
        while self.buf.front().is_some_and(|x| x.0 < cutoff) {
            self.buf.pop_front();
        }
    }
}
