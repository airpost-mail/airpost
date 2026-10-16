// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Tern contributors

//! Random identifiers, device tokens and timestamps (docs/backend-plan.md 3.3).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

pub const TOKEN_PREFIX: &str = "tern_";

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    rand::fill(&mut buf[..]);
    buf
}

/// `users.id` and `identity.id`: 24 lowercase hex chars (96 bits).
pub fn new_user_id() -> String {
    hex(&random_bytes::<12>())
}

/// `meta.server_id`: 32 lowercase hex chars.
pub fn new_server_id() -> String {
    hex(&random_bytes::<16>())
}

/// `users.tracking_ns`: 128 random bits, base64url, 22 chars.
pub fn new_tracking_ns() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<16>())
}

/// A device token, `tern_` + base64url(32 random bytes). Only [`hash_token`] of it is stored.
pub fn new_token() -> String {
    format!(
        "{TOKEN_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(random_bytes::<32>())
    )
}

/// A password for `user add` when none is given: 18 random bytes, base64url, 24 chars.
pub fn new_password() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<18>())
}

pub fn hash_token(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// UTC timestamps in CLI listings, `%Y-%m-%dT%H:%M:%S.000Z`.
pub fn format_time(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S.000Z")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let u = new_user_id();
        assert_eq!(u.len(), 24);
        assert!(u
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_ne!(u, new_user_id());

        let ns = new_tracking_ns();
        assert_eq!(ns.len(), 22);
        assert!(ns
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));

        let t = new_token();
        assert!(t.starts_with("tern_"));
        assert_eq!(t.len(), 5 + 43);
        assert_ne!(hash_token(&t), hash_token(&new_token()));
        assert_eq!(new_server_id().len(), 32);
        assert_eq!(new_password().len(), 24);
    }

    #[test]
    fn created_at_format() {
        assert_eq!(format_time(1_518_652_800_123), "2018-02-15T00:00:00.000Z");
        assert_eq!(format_time(0), "1970-01-01T00:00:00.000Z");
    }
}
