use std::collections::VecDeque;

use chrono::{DateTime, Local};

#[derive(Debug, Clone)]
pub struct Timeline {
    bins: VecDeque<Bin>,
    bin_width: chrono::Duration,
    last_bin_start: DateTime<Local>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Bin {
    pub info: u64,
    pub warn: u64,
    pub error: u64,
}

impl Timeline {
    pub fn new(bin_count: usize, window: std::time::Duration) -> Self {
        let total_secs = window.as_secs().max(1);
        let bin_secs = (total_secs / bin_count.max(1) as u64).max(1);
        let bin_width = chrono::Duration::seconds(bin_secs as i64);
        let now = Local::now();
        Self {
            bins: VecDeque::from(vec![Bin::default(); bin_count.max(1)]),
            bin_width,
            last_bin_start: now - bin_width,
        }
    }

    pub fn record(&mut self, now: DateTime<Local>, info: u64, warn: u64, error: u64) {
        self.advance(now);
        if let Some(last) = self.bins.back_mut() {
            last.info += info;
            last.warn += warn;
            last.error += error;
        }
    }

    pub fn advance(&mut self, now: DateTime<Local>) {
        if self.bins.is_empty() {
            return;
        }
        while now - self.last_bin_start >= self.bin_width {
            self.bins.pop_front();
            self.bins.push_back(Bin::default());
            self.last_bin_start += self.bin_width;
        }
    }

    pub fn data(&self) -> Vec<Bin> {
        self.bins.iter().copied().collect()
    }

    pub fn range(&self) -> (DateTime<Local>, DateTime<Local>) {
        if self.bins.is_empty() {
            let now = Local::now();
            return (now, now);
        }
        let span = self.bin_width * (self.bins.len() as i32);
        let end = self.last_bin_start + self.bin_width;
        let start = end - span;
        (start, end)
    }

    pub fn len(&self) -> usize {
        self.bins.len()
    }

    pub fn bin_start(&self, idx_from_oldest: usize) -> DateTime<Local> {
        let (start, _) = self.range();
        start + self.bin_width * (idx_from_oldest as i32)
    }

    pub fn bin_index_for(&self, ts: DateTime<Local>) -> Option<usize> {
        let (start, end) = self.range();
        if ts < start || ts >= end {
            return None;
        }
        let offset = ts - start;
        let secs = offset.num_seconds();
        let bin_secs = self.bin_width.num_seconds().max(1);
        let idx = (secs / bin_secs) as usize;
        if idx < self.bins.len() {
            Some(idx)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timeline_tracks_bins() {
        let mut timeline = Timeline::new(5, Duration::from_secs(5));
        let now = Local::now();
        timeline.record(now, 3, 1, 0);
        timeline.record(now + chrono::Duration::seconds(6), 2, 0, 1);
        assert_eq!(timeline.data().len(), 5);
        assert!(timeline
            .data()
            .iter()
            .any(|v| v.info + v.warn + v.error >= 2));
    }
}
