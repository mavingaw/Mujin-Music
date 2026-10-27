//! Clock arithmetic for sample-synced playback.
//!
//! - [`ClockEstimator`]: the offset between the server's clock and a
//!   bridge's, from ping round trips (NTP style: the sample with the
//!   shortest round trip is the most trustworthy).
//! - [`RateEstimator`]: how fast a DAC really plays (its crystal is never
//!   exactly 44 100 Hz), from (time, frames heard) reports.
//! - [`drift_correction`]: what a follower does to stay on the clock
//!   master's timeline: nothing inside the dead band, one frame per write
//!   (about 1000 ppm at most, inaudible) outside it, a jump when far off.

use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::Instant;

/// Microseconds on this process's monotonic clock.
pub fn monotonic_us() -> i64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    // Start well above zero so offsets never make times negative.
    1_000_000_000 + EPOCH.get_or_init(Instant::now).elapsed().as_micros() as i64
}

const SAMPLES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Sample {
    rtt_us: i64,
    offset_us: i64,
}

/// Offset of a remote clock from ours.
#[derive(Clone, Debug, Default)]
pub struct ClockEstimator {
    samples: VecDeque<Sample>,
}

impl ClockEstimator {
    /// A ping sent at `sent_us` (ours) answered with the remote time
    /// `remote_us`, received at `received_us` (ours).
    pub fn add(&mut self, sent_us: i64, remote_us: i64, received_us: i64) {
        let rtt_us = (received_us - sent_us).max(0);
        let offset_us = remote_us - (sent_us + rtt_us / 2);
        if self.samples.len() == SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(Sample { rtt_us, offset_us });
    }

    fn best(&self) -> Option<Sample> {
        self.samples
            .iter()
            .copied()
            .min_by_key(|sample| sample.rtt_us)
    }

    /// Remote minus local, in microseconds.
    pub fn offset_us(&self) -> Option<i64> {
        self.best().map(|sample| sample.offset_us)
    }

    /// Half the best round trip: how far off the offset can be.
    pub fn uncertainty_us(&self) -> Option<i64> {
        self.best().map(|sample| sample.rtt_us / 2)
    }

    pub fn to_remote(&self, local_us: i64) -> i64 {
        local_us + self.offset_us().unwrap_or(0)
    }

    pub fn to_local(&self, remote_us: i64) -> i64 {
        remote_us - self.offset_us().unwrap_or(0)
    }
}

/// A DAC's real playback rate, from progress reports.
#[derive(Clone, Debug)]
pub struct RateEstimator {
    nominal: f64,
    points: VecDeque<(i64, u64)>,
    window_us: i64,
}

impl RateEstimator {
    pub fn new(nominal: u32) -> Self {
        Self {
            nominal: f64::from(nominal),
            points: VecDeque::new(),
            window_us: 8_000_000,
        }
    }

    /// `frames` were heard by `at_us`.
    pub fn add(&mut self, at_us: i64, frames: u64) {
        if let Some(&(last_at, last_frames)) = self.points.back()
            && (at_us <= last_at || frames < last_frames)
        {
            if frames < last_frames {
                self.points.clear();
            } else {
                return;
            }
        }
        self.points.push_back((at_us, frames));
        while let Some(&(first_at, _)) = self.points.front() {
            if at_us - first_at > self.window_us && self.points.len() > 2 {
                self.points.pop_front();
            } else {
                break;
            }
        }
    }

    /// Frames per second, from at least a second of reports (else nominal);
    /// never more than 0.5% from nominal, so one bad report cannot run away.
    pub fn rate(&self) -> f64 {
        let (Some(&(first_at, first)), Some(&(last_at, last))) =
            (self.points.front(), self.points.back())
        else {
            return self.nominal;
        };
        let span = last_at - first_at;
        if span < 1_000_000 || last <= first {
            return self.nominal;
        }
        let measured = (last - first) as f64 * 1e6 / span as f64;
        measured.clamp(self.nominal * 0.995, self.nominal * 1.005)
    }

    /// Latest (time, frames) report.
    pub fn latest(&self) -> Option<(i64, u64)> {
        self.points.back().copied()
    }
}

/// What a follower does before its next write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Correction {
    None,
    /// Play this many extra frames (repeat or silence): it is ahead.
    Insert(u64),
    /// Skip this many source frames: it is behind.
    Drop(u64),
}

/// `error_frames` = heard - target (positive: ahead of the master).
pub fn drift_correction(error_frames: f64, sample_rate: u32) -> Correction {
    let rate = f64::from(sample_rate);
    // Dead band: a quarter millisecond.
    let dead_band = (rate / 4_000.0).max(2.0);
    // Beyond 20 ms, jump instead of slewing.
    let jump = rate / 50.0;
    if error_frames.abs() <= dead_band {
        Correction::None
    } else if error_frames > jump {
        Correction::Insert(error_frames.round() as u64)
    } else if error_frames < -jump {
        Correction::Drop((-error_frames).round() as u64)
    } else if error_frames > 0.0 {
        Correction::Insert(1)
    } else {
        Correction::Drop(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_comes_from_the_shortest_round_trip() {
        let mut clock = ClockEstimator::default();
        assert_eq!(clock.offset_us(), None);
        // Remote clock is 5 s ahead. A slow, asymmetric ping first.
        clock.add(1_000, 5_000_000 + 1_000 + 9_000, 1_000 + 12_000);
        clock.add(20_000, 5_000_000 + 20_000 + 150, 20_000 + 300);
        clock.add(40_000, 5_000_000 + 40_000 + 4_000, 40_000 + 5_000);
        assert_eq!(clock.offset_us(), Some(5_000_000));
        assert_eq!(clock.uncertainty_us(), Some(150));
        assert_eq!(clock.to_remote(1), 5_000_001);
        assert_eq!(clock.to_local(5_000_001), 1);
    }

    #[test]
    fn rate_follows_a_dac_that_runs_fast() {
        let mut rate = RateEstimator::new(44_100);
        assert_eq!(rate.rate(), 44_100.0);
        // 300 ppm fast over 3 s.
        for tenth in 0..=30_i64 {
            let frames = (44_100.0 * 1.0003 * tenth as f64 / 10.0) as u64;
            rate.add(tenth * 100_000, frames);
        }
        assert!((rate.rate() - 44_113.23).abs() < 0.5, "{}", rate.rate());
        // A bogus report is clamped.
        rate.add(3_100_000, 10_000_000);
        assert!(rate.rate() <= 44_100.0 * 1.005);
        assert_eq!(rate.latest(), Some((3_100_000, 10_000_000)));
    }

    #[test]
    fn corrections_slew_inside_twenty_ms_and_jump_beyond() {
        assert_eq!(drift_correction(5.0, 44_100), Correction::None);
        assert_eq!(drift_correction(-11.0, 44_100), Correction::None);
        assert_eq!(drift_correction(30.0, 44_100), Correction::Insert(1));
        assert_eq!(drift_correction(-30.0, 44_100), Correction::Drop(1));
        assert_eq!(drift_correction(4_410.0, 44_100), Correction::Insert(4_410));
        assert_eq!(drift_correction(-9_600.0, 192_000), Correction::Drop(9_600));
    }
}
