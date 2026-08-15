//! Staying inside the Nexus API rate limits.
//!
//! Every response carries the remaining allowance in headers. Reading them is
//! cheaper than being cut off, and showing the number in the UI explains why a
//! big collection install slows down instead of leaving the user guessing.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The allowance as of the last response.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimit {
    pub hourly_remaining: Option<i64>,
    pub hourly_limit: Option<i64>,
    pub daily_remaining: Option<i64>,
    pub daily_limit: Option<i64>,
    /// Seconds until the hourly window resets, when the server said so.
    pub reset_after_secs: Option<u64>,
}

impl RateLimit {
    /// Read the `X-RL-*` headers. Names are matched case-insensitively because
    /// header casing is not guaranteed.
    pub fn from_headers<'a, I>(headers: I) -> Self
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut limit = RateLimit::default();
        for (name, value) in headers {
            let lower = name.to_ascii_lowercase();
            let number = value.trim().parse::<i64>().ok();
            match lower.as_str() {
                "x-rl-hourly-remaining" => limit.hourly_remaining = number,
                "x-rl-hourly-limit" => limit.hourly_limit = number,
                "x-rl-daily-remaining" => limit.daily_remaining = number,
                "x-rl-daily-limit" => limit.daily_limit = number,
                "retry-after" => limit.reset_after_secs = value.trim().parse().ok(),
                _ => {}
            }
        }
        limit
    }

    /// True when the next call would almost certainly be refused.
    pub fn is_exhausted(&self) -> bool {
        matches!(self.hourly_remaining, Some(n) if n <= 0)
            || matches!(self.daily_remaining, Some(n) if n <= 0)
    }

    /// Merge a fresh reading over the previous one, keeping known values.
    pub fn merge(&mut self, newer: RateLimit) {
        if newer.hourly_remaining.is_some() {
            self.hourly_remaining = newer.hourly_remaining;
        }
        if newer.hourly_limit.is_some() {
            self.hourly_limit = newer.hourly_limit;
        }
        if newer.daily_remaining.is_some() {
            self.daily_remaining = newer.daily_remaining;
        }
        if newer.daily_limit.is_some() {
            self.daily_limit = newer.daily_limit;
        }
        if newer.reset_after_secs.is_some() {
            self.reset_after_secs = newer.reset_after_secs;
        }
    }
}

/// How long to wait before retrying attempt number `attempt` (1-based).
///
/// A `Retry-After` from the server always wins; otherwise this backs off
/// exponentially and stops growing at a minute, so a long queue recovers on
/// its own without hammering the API.
pub fn backoff(attempt: u32, retry_after: Option<u64>) -> Duration {
    if let Some(secs) = retry_after {
        return Duration::from_secs(secs.min(3600));
    }
    let seconds = 2u64.saturating_pow(attempt.min(6));
    Duration::from_secs(seconds.min(60))
}

/// Whether a status is worth trying again.
pub fn is_retryable(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_allowance_headers() {
        let limit = RateLimit::from_headers([
            ("X-RL-Hourly-Remaining", "97"),
            ("X-RL-Hourly-Limit", "100"),
            ("x-rl-daily-remaining", "2400"),
        ]);
        assert_eq!(limit.hourly_remaining, Some(97));
        assert_eq!(limit.hourly_limit, Some(100));
        assert_eq!(limit.daily_remaining, Some(2400));
        assert!(!limit.is_exhausted());
    }

    #[test]
    fn zero_remaining_counts_as_exhausted() {
        let limit = RateLimit::from_headers([("X-RL-Hourly-Remaining", "0")]);
        assert!(limit.is_exhausted());
    }

    #[test]
    fn merging_keeps_values_the_newer_reading_did_not_mention() {
        let mut limit = RateLimit::from_headers([("X-RL-Daily-Limit", "2500")]);
        limit.merge(RateLimit::from_headers([("X-RL-Hourly-Remaining", "5")]));

        assert_eq!(
            limit.daily_limit,
            Some(2500),
            "the older value must survive"
        );
        assert_eq!(limit.hourly_remaining, Some(5));
    }

    #[test]
    fn the_servers_retry_after_wins_over_our_backoff() {
        assert_eq!(backoff(1, Some(30)), Duration::from_secs(30));
    }

    #[test]
    fn backoff_grows_then_levels_off() {
        assert_eq!(backoff(1, None), Duration::from_secs(2));
        assert_eq!(backoff(3, None), Duration::from_secs(8));
        assert_eq!(backoff(10, None), Duration::from_secs(60));
    }

    #[test]
    fn only_throttling_and_server_faults_are_retried() {
        assert!(is_retryable(429));
        assert!(is_retryable(503));
        assert!(!is_retryable(404));
        assert!(!is_retryable(403), "a bad key will not fix itself");
    }
}
