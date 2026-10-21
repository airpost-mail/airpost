// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! [`IntegrationError`]: every provider failure, with a stable key for the client.

use std::time::Duration;

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

pub type Result<T, E = IntegrationError> = std::result::Result<T, E>;

/// A failed provider call. [`IntegrationError::key`] is stable and is what the desktop
/// switches on; the `Display` text is for people and never contains a key, a username, the
/// request text or an email address. Provider messages are carried truncated.
///
/// Serializes as `{key, message, retryable, status?, retryAfter?}` (`retryAfter` in
/// seconds), the `data` of the socket's error response.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum IntegrationError {
    /// A required setting (URL, key) is missing or blank.
    #[error("{what} is not set")]
    NotConfigured { what: &'static str },
    /// The provider rejected the key or account.
    #[error("the provider rejected the credentials (HTTP {status})")]
    Auth { status: u16 },
    #[error("the provider is rate limiting requests")]
    RateLimited { retry_after: Option<Duration> },
    /// The account's character or credit allowance is used up.
    #[error("the provider account's quota is used up (HTTP {status})")]
    QuotaExceeded { status: u16 },
    /// No response: DNS, connect, TLS or timeout. `reason` is the failure class, never the
    /// URL (the URL can carry an email address).
    #[error("could not reach the provider ({reason})")]
    Offline { reason: &'static str },
    /// The provider refused this request (or the URL setting is malformed); sending it
    /// again won't help.
    #[error("the provider refused the request: {message}")]
    BadRequest {
        status: Option<u16>,
        message: String,
    },
    /// The provider failed (5xx) or answered with something this crate can't read.
    #[error("the provider failed (HTTP {status}): {message}")]
    Provider { status: u16, message: String },
    /// The provider has no such operation (language detection on DeepL).
    #[error("{provider} does not support {operation}")]
    Unsupported {
        provider: &'static str,
        operation: &'static str,
    },
}

impl IntegrationError {
    pub fn key(&self) -> &'static str {
        match self {
            IntegrationError::NotConfigured { .. } => "not-configured",
            IntegrationError::Auth { .. } => "auth",
            IntegrationError::RateLimited { .. } => "rate-limited",
            IntegrationError::QuotaExceeded { .. } => "quota-exceeded",
            IntegrationError::Offline { .. } => "offline",
            IntegrationError::BadRequest { .. } => "bad-request",
            IntegrationError::Provider { .. } => "provider-error",
            IntegrationError::Unsupported { .. } => "unsupported",
        }
    }

    /// Whether the same request may succeed later without the user changing anything.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            IntegrationError::RateLimited { .. }
                | IntegrationError::Offline { .. }
                | IntegrationError::Provider { .. }
        )
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            IntegrationError::RateLimited { retry_after } => *retry_after,
            _ => None,
        }
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            IntegrationError::Auth { status }
            | IntegrationError::QuotaExceeded { status }
            | IntegrationError::Provider { status, .. } => Some(*status),
            IntegrationError::BadRequest { status, .. } => *status,
            _ => None,
        }
    }

    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        IntegrationError::BadRequest {
            status: None,
            message: message.into(),
        }
    }

    /// A 2xx body that doesn't have the documented shape.
    pub(crate) fn unreadable(status: u16) -> Self {
        IntegrationError::Provider {
            status,
            message: "unexpected response body".into(),
        }
    }
}

impl Serialize for IntegrationError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let status = self.status();
        let retry_after = self.retry_after();
        let len = 3 + usize::from(status.is_some()) + usize::from(retry_after.is_some());
        let mut s = serializer.serialize_struct("IntegrationError", len)?;
        s.serialize_field("key", self.key())?;
        s.serialize_field("message", &self.to_string())?;
        s.serialize_field("retryable", &self.is_retryable())?;
        if let Some(status) = status {
            s.serialize_field("status", &status)?;
        }
        if let Some(after) = retry_after {
            s.serialize_field("retryAfter", &after.as_secs())?;
        }
        s.end()
    }
}
