// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! RFC 6585 (429) and RFC 9110 §10.2.3 (Retry-After) backoff, shared by CalDAV and CardDAV
//! requests. Derived from Mailspring-Sync `MailSync/DAVWorker.cpp` (`parseRetryAfter`,
//! `applyRateLimitDelay`, `recordRequestSuccess`, `recordRateLimitResponse`), Copyright ©
//! 2017 Foundry 376, GPL-3.0. See NOTICE.md.

use chrono::NaiveDateTime;
use std::time::Duration;
use tracing::{info, warn};

/// The longest exponential backoff.
pub const MAX_BACKOFF_MS: u64 = 60_000;
/// The first backoff step; a halved backoff below it clears.
pub const MIN_BACKOFF_MS: u64 = 100;
/// Successes in a row that halve the backoff.
pub const SUCCESSES_TO_HALVE: u32 = 3;

/// Seconds to wait from a Retry-After value, 0 when it gives none. Accepts delay-seconds
/// (leading digits, as `stoi` read them) and the IMF-fixdate form
/// `Fri, 31 Dec 2024 23:59:59 GMT`, read as UTC.
pub fn parse_retry_after(value: &str, now: i64) -> i64 {
    let value = value.trim();
    if value.is_empty() {
        return 0;
    }
    let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
    if let Ok(secs) = digits.parse::<i64>() {
        if secs > 0 {
            return secs;
        }
    }
    // The weekday is skipped rather than checked, as strptime's %a did.
    let date = value
        .split_once(',')
        .map_or(value, |(_, rest)| rest.trim_start());
    match NaiveDateTime::parse_and_remainder(date, "%d %b %Y %H:%M:%S") {
        Ok((at, _)) => (at.and_utc().timestamp() - now).max(0),
        Err(_) => 0,
    }
}

/// The waits to apply before the next request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Delay {
    /// Until the time a Retry-After named.
    pub retry_after: Option<Duration>,
    /// The exponential backoff after a 429/503 without Retry-After.
    pub backoff: Option<Duration>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RateLimiter {
    backoff_ms: u64,
    consecutive_successes: u32,
    limited_until: i64,
}

impl RateLimiter {
    pub fn backoff_ms(&self) -> u64 {
        self.backoff_ms
    }

    pub fn limited_until(&self) -> i64 {
        self.limited_until
    }

    /// What to wait before a request at `now` (`applyRateLimitDelay`), logging each wait.
    pub fn delay(&self, now: i64) -> Delay {
        let mut d = Delay::default();
        if self.limited_until > now {
            let secs = self.limited_until - now;
            info!("Rate limited: waiting {secs} seconds (Retry-After)");
            d.retry_after = Some(Duration::from_secs(secs as u64));
        }
        if self.backoff_ms > 0 {
            info!("Rate limit backoff: waiting {}ms", self.backoff_ms);
            d.backoff = Some(Duration::from_millis(self.backoff_ms));
        }
        d
    }

    /// `recordRequestSuccess`: every third success in a row halves the backoff.
    pub fn record_success(&mut self) {
        self.consecutive_successes += 1;
        if self.consecutive_successes >= SUCCESSES_TO_HALVE && self.backoff_ms > 0 {
            self.backoff_ms /= 2;
            self.consecutive_successes = 0;
            if self.backoff_ms < MIN_BACKOFF_MS {
                self.backoff_ms = 0;
                info!("Rate limit backoff cleared");
            }
        }
    }

    /// `recordRateLimitResponse` for a 429 or 503: honour Retry-After when it names a wait,
    /// else double the backoff (starting at [`MIN_BACKOFF_MS`], capped at [`MAX_BACKOFF_MS`]).
    pub fn record_rate_limited(&mut self, status: u16, retry_after: Option<&str>, now: i64) {
        self.consecutive_successes = 0;
        let secs = retry_after.map_or(0, |v| parse_retry_after(v, now));
        if secs > 0 {
            self.limited_until = now + secs;
            warn!("Rate limited ({status}): server requested {secs} second wait");
        } else {
            self.backoff_ms = if self.backoff_ms == 0 {
                MIN_BACKOFF_MS
            } else {
                (self.backoff_ms * 2).min(MAX_BACKOFF_MS)
            };
            warn!(
                "Rate limited ({status}): using exponential backoff ({}ms)",
                self.backoff_ms
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_forms() {
        let now = 1_735_689_000; // 2024-12-31T23:50:00Z
        assert_eq!(parse_retry_after("120", now), 120);
        assert_eq!(parse_retry_after(" 5 ", now), 5);
        assert_eq!(parse_retry_after("0", now), 0);
        assert_eq!(parse_retry_after("", now), 0);
        assert_eq!(parse_retry_after("Tue, 31 Dec 2024 23:59:59 GMT", now), 599);
        assert_eq!(parse_retry_after("Tue, 31 Dec 2024 23:00:00 GMT", now), 0);
        assert_eq!(parse_retry_after("Fri, 31 Dec 2024 23:59:59 GMT", now), 599);
        assert_eq!(parse_retry_after("soon", now), 0);
    }

    #[test]
    fn backoff_doubles_caps_and_decays() {
        let mut r = RateLimiter::default();
        r.record_rate_limited(429, None, 0);
        assert_eq!(r.backoff_ms(), 100);
        for _ in 0..12 {
            r.record_rate_limited(503, Some(""), 0);
        }
        assert_eq!(r.backoff_ms(), MAX_BACKOFF_MS);
        for _ in 0..3 {
            r.record_success();
        }
        assert_eq!(r.backoff_ms(), 30_000);
        while r.backoff_ms() > 0 {
            for _ in 0..3 {
                r.record_success();
            }
        }
        assert_eq!(r.delay(0), Delay::default());
    }

    #[test]
    fn retry_after_blocks_until_then() {
        let mut r = RateLimiter::default();
        r.record_rate_limited(429, Some("30"), 1000);
        assert_eq!(r.limited_until(), 1030);
        assert_eq!(r.delay(1010).retry_after, Some(Duration::from_secs(20)));
        assert_eq!(r.delay(1030).retry_after, None);
        assert_eq!(r.backoff_ms(), 0);
    }
}
