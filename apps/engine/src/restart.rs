// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `serve`'s restart policy for a stopped account: refused
//! credentials stop it until the client adds it again (`authFailed`); otherwise it is
//! started again after a backoff (`retryAt`), and [`CRASH_LIMIT`] stops in
//! [`CRASH_WINDOW`] stop it for good, as the desktop's `CrashTracker` did.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde_json::Value;

pub const CRASH_LIMIT: usize = 5;
pub const CRASH_WINDOW: Duration = Duration::from_secs(5 * 60);
/// The first retry's delay; each later one doubles it, up to [`MAX_DELAY`].
pub const BASE_DELAY: Duration = Duration::from_secs(5);
pub const MAX_DELAY: Duration = Duration::from_secs(5 * 60);
/// Overrides [`BASE_DELAY`], in milliseconds (tests).
pub const ENV_BASE_DELAY: &str = "TERN_ENGINE_RESTART_DELAY_MS";

/// The SyncError keys that mean the server refused the credentials.
const AUTH_KEYS: [&str; 5] = [
    "ErrorAuthentication",
    "ErrorAuthenticationRequired",
    "ErrorGmailApplicationSpecificPasswordRequired",
    "ErrorOutlookLoginViaWebBrowser",
    "ErrorTiscaliSimplePassword",
];
/// The token manager's key for a refresh the provider answered with an error.
const OAUTH_KEY: &str = "invalid-xoauth2-resp";

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    AuthFailed,
    GiveUp,
    RetryIn(Duration),
}

/// One account's recent stops.
#[derive(Debug, Default)]
pub struct History {
    stops: VecDeque<Instant>,
}

impl History {
    /// Records a stop at `now` with `error` (the `account.state` error) and decides what
    /// follows.
    pub fn record(&mut self, error: Option<&Value>, now: Instant, base: Duration) -> Decision {
        if error.is_some_and(is_auth_failure) {
            return Decision::AuthFailed;
        }
        self.stops.push_back(now);
        while self
            .stops
            .front()
            .is_some_and(|t| now.duration_since(*t) > CRASH_WINDOW)
        {
            self.stops.pop_front();
        }
        if self.stops.len() >= CRASH_LIMIT {
            return Decision::GiveUp;
        }
        let doublings = u32::try_from(self.stops.len().saturating_sub(1)).unwrap_or(u32::MAX);
        let delay = base
            .checked_mul(2u32.saturating_pow(doublings))
            .unwrap_or(MAX_DELAY)
            .min(MAX_DELAY.max(base));
        Decision::RetryIn(delay)
    }
}

/// The base delay, from [`ENV_BASE_DELAY`] when it is set.
pub fn base_delay() -> Duration {
    std::env::var(ENV_BASE_DELAY)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(BASE_DELAY, Duration::from_millis)
}

/// Whether a stop's error is the server refusing the account's credentials.
pub fn is_auth_failure(error: &Value) -> bool {
    let key = error.get("key").and_then(Value::as_str).unwrap_or_default();
    if AUTH_KEYS.contains(&key) {
        return true;
    }
    let retryable = error
        .get("retryable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let info = error
        .get("debuginfo")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if key == OAUTH_KEY && !retryable {
        return true;
    }
    info.contains("invalid_grant")
        || info.contains("Response Code: 401")
        || info.contains("Response Code: 403")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn auth_failures_are_classified() {
        assert!(is_auth_failure(&json!({"key": "ErrorAuthentication"})));
        assert!(is_auth_failure(
            &json!({"key": "invalid-xoauth2-resp", "retryable": false})
        ));
        assert!(!is_auth_failure(
            &json!({"key": "invalid-xoauth2-resp", "retryable": true})
        ));
        assert!(is_auth_failure(
            &json!({"key": "x", "debuginfo": "{\"error\":\"invalid_grant\"}"})
        ));
        assert!(!is_auth_failure(&json!({"key": "ErrorConnection"})));
        assert!(!is_auth_failure(&json!({"key": "unhandled-exception"})));
    }

    #[test]
    fn backoff_doubles_and_five_stops_in_five_minutes_give_up() {
        let mut h = History::default();
        let t = Instant::now();
        let base = Duration::from_secs(5);
        let crash = json!({"key": "unhandled-exception"});
        let delays: Vec<_> = (0..4)
            .map(|i| h.record(Some(&crash), t + Duration::from_secs(i), base))
            .collect();
        assert_eq!(
            delays,
            [5, 10, 20, 40].map(|s| Decision::RetryIn(Duration::from_secs(s)))
        );
        assert_eq!(
            h.record(Some(&crash), t + Duration::from_secs(5), base),
            Decision::GiveUp
        );
        // Stops older than the window no longer count.
        let mut h = History::default();
        for i in 0..4 {
            h.record(None, t + Duration::from_secs(i * 100), base);
        }
        assert!(matches!(
            h.record(None, t + Duration::from_secs(400), base),
            Decision::RetryIn(_)
        ));
        assert_eq!(
            h.record(Some(&json!({"key": "ErrorAuthentication"})), t, base),
            Decision::AuthFailed
        );
    }
}
