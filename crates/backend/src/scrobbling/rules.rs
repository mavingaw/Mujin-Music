//! Scrobbling rules shared by Last.fm and ListenBrainz.
//!
//! Last.fm's published rules (which ListenBrainz follows): a track is
//! scrobbled only when it is longer than 30 seconds and has been played for
//! at least half its duration or for 4 minutes, whichever comes first. The
//! timestamp is the moment playback of the track started.

/// Tracks this short or shorter are never scrobbled.
pub const MIN_TRACK_SECONDS: f64 = 30.0;
/// Playing this long always qualifies, however long the track is.
pub const MAX_THRESHOLD_SECONDS: f64 = 240.0;
/// First retry delay of the offline queue.
pub const BASE_RETRY_SECONDS: i64 = 30;
/// Longest gap between two retries.
pub const MAX_RETRY_SECONDS: i64 = 6 * 60 * 60;
/// A listen that still fails after this many attempts is given up on
/// (with the 6 hour cap that is more than a week of retrying).
pub const MAX_ATTEMPTS: i32 = 40;
/// Last.fm ignores scrobbles older than 14 days.
pub const LASTFM_MAX_AGE_SECONDS: i64 = 14 * 24 * 60 * 60;

/// Seconds of listening a track of `duration` needs before it scrobbles.
pub fn threshold_seconds(duration: f64) -> f64 {
    (duration / 2.0).min(MAX_THRESHOLD_SECONDS)
}

/// Whether a track of `duration` seconds may be scrobbled at all.
pub fn long_enough(duration: f64) -> bool {
    duration.is_finite() && duration > MIN_TRACK_SECONDS
}

/// Whether `listened` seconds of a `duration` second track is a scrobble.
pub fn qualifies(duration: f64, listened: f64) -> bool {
    long_enough(duration) && listened.is_finite() && listened >= threshold_seconds(duration)
}

/// The listen timestamp: when playback started. Falls back to "now minus
/// the playback position" when the start was not observed, and never lies
/// in the future.
pub fn listened_at(started_at: Option<i64>, now: i64, position_seconds: f64) -> i64 {
    let estimated = if position_seconds.is_finite() && position_seconds > 0.0 {
        now - position_seconds.round() as i64
    } else {
        now
    };
    started_at.unwrap_or(estimated).min(now)
}

/// Delay before retry number `attempts` (1-based): 30 s, 1 min, 2 min, ...
/// capped at 6 hours.
pub fn retry_delay_seconds(attempts: i32) -> i64 {
    let exponent = attempts.saturating_sub(1).clamp(0, 20) as u32;
    BASE_RETRY_SECONDS
        .saturating_mul(1_i64 << exponent)
        .min(MAX_RETRY_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_of_thirty_seconds_or_less_never_scrobble() {
        assert!(!qualifies(30.0, 30.0));
        assert!(!qualifies(12.0, 12.0));
        assert!(qualifies(31.0, 15.5));
        assert!(!long_enough(f64::NAN));
    }

    #[test]
    fn half_the_track_or_four_minutes_whichever_comes_first() {
        // A 3 minute track needs 90 seconds.
        assert!(!qualifies(180.0, 89.9));
        assert!(qualifies(180.0, 90.0));
        // A 20 minute track needs 4 minutes, not 10.
        assert!(!qualifies(1200.0, 239.0));
        assert!(qualifies(1200.0, 240.0));
        assert_eq!(threshold_seconds(1200.0), 240.0);
        assert_eq!(threshold_seconds(300.0), 150.0);
    }

    #[test]
    fn timestamps_use_the_start_of_playback() {
        assert_eq!(listened_at(Some(1_000), 1_200, 150.0), 1_000);
        assert_eq!(listened_at(None, 1_200, 150.0), 1_050);
        assert_eq!(listened_at(None, 1_200, f64::NAN), 1_200);
        // A start reported in the future is clamped to now.
        assert_eq!(listened_at(Some(5_000), 1_200, 0.0), 1_200);
    }

    #[test]
    fn retries_back_off_exponentially_up_to_six_hours() {
        assert_eq!(retry_delay_seconds(1), 30);
        assert_eq!(retry_delay_seconds(2), 60);
        assert_eq!(retry_delay_seconds(5), 480);
        assert_eq!(retry_delay_seconds(30), MAX_RETRY_SECONDS);
        assert_eq!(retry_delay_seconds(0), 30);
    }
}
