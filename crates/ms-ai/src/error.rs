// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! [`AiError`]: every failure of a model call, with a stable key for the client.

use std::time::Duration;

use ms_integrations::IntegrationError;
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

pub type Result<T, E = AiError> = std::result::Result<T, E>;

/// A failed model call. [`AiError::key`] is stable and is what clients switch on; the
/// `Display` text is for people and never contains a key, an address or mail text.
/// Provider messages are carried truncated.
///
/// Serializes like `IntegrationError`, as `{key, message, retryable, status?, retryAfter?}`
/// (`retryAfter` in seconds): the `data` of the socket's `-32011` error.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum AiError {
    /// A required setting (URL, key, model) is missing or blank.
    #[error("{what} is not set")]
    NotConfigured { what: &'static str },
    /// The account isn't opted in or the feature is off. Raised by the engine, which
    /// checks both before it reads any mail.
    #[error("{what} is turned off")]
    NotEnabled { what: &'static str },
    #[error("the provider rejected the credentials (HTTP {status})")]
    Auth { status: u16 },
    #[error("the provider is rate limiting requests")]
    RateLimited { retry_after: Option<Duration> },
    /// The provider account's credit is used up, or the engine's daily token cap is reached
    /// (no status; `retry_after` runs to midnight).
    #[error("the token quota is used up")]
    QuotaExceeded {
        status: Option<u16>,
        retry_after: Option<Duration>,
    },
    /// No response, or none in time: DNS, connect, TLS or a timeout. `reason` is the failure
    /// class, never the URL.
    #[error("could not reach the provider ({reason})")]
    Offline { reason: &'static str },
    /// The provider refused this request (or a setting is malformed); sending it again
    /// won't help.
    #[error("the provider refused the request: {message}")]
    BadRequest {
        status: Option<u16>,
        message: String,
    },
    #[error("the provider has no such model: {message}")]
    ModelNotFound {
        status: Option<u16>,
        message: String,
    },
    /// The thread didn't fit the model's context window even after budgeting; lower
    /// `contextTokens` to match the model.
    #[error("the request is too long for the model: {message}")]
    ContextTooLong {
        status: Option<u16>,
        message: String,
    },
    /// The provider failed (5xx, a stream error event, a cut stream) or answered with
    /// something this crate can't read.
    #[error("the provider failed: {message}")]
    Provider {
        status: Option<u16>,
        message: String,
    },
    /// The caller cancelled the request (`ai.cancel`, a closed connection).
    #[error("the request was cancelled")]
    Cancelled,
    #[error("{what} is not supported")]
    Unsupported { what: &'static str },
}

impl AiError {
    pub fn key(&self) -> &'static str {
        match self {
            AiError::NotConfigured { .. } => "not-configured",
            AiError::NotEnabled { .. } => "not-enabled",
            AiError::Auth { .. } => "auth",
            AiError::RateLimited { .. } => "rate-limited",
            AiError::QuotaExceeded { .. } => "quota-exceeded",
            AiError::Offline { .. } => "offline",
            AiError::BadRequest { .. } => "bad-request",
            AiError::ModelNotFound { .. } => "model-not-found",
            AiError::ContextTooLong { .. } => "context-too-long",
            AiError::Provider { .. } => "provider-error",
            AiError::Cancelled => "cancelled",
            AiError::Unsupported { .. } => "unsupported",
        }
    }

    /// Whether the same request may succeed later without the user changing anything.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            AiError::RateLimited { .. } | AiError::Offline { .. } | AiError::Provider { .. }
        )
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            AiError::RateLimited { retry_after } | AiError::QuotaExceeded { retry_after, .. } => {
                *retry_after
            }
            _ => None,
        }
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            AiError::Auth { status } => Some(*status),
            AiError::QuotaExceeded { status, .. }
            | AiError::BadRequest { status, .. }
            | AiError::ModelNotFound { status, .. }
            | AiError::ContextTooLong { status, .. }
            | AiError::Provider { status, .. } => *status,
            _ => None,
        }
    }

    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        AiError::BadRequest {
            status: None,
            message: message.into(),
        }
    }

    pub(crate) fn provider(message: impl Into<String>) -> Self {
        AiError::Provider {
            status: None,
            message: message.into(),
        }
    }
}

impl From<IntegrationError> for AiError {
    fn from(e: IntegrationError) -> Self {
        match e {
            IntegrationError::NotConfigured { what } => AiError::NotConfigured { what },
            IntegrationError::Auth { status } => AiError::Auth { status },
            IntegrationError::RateLimited { retry_after } => AiError::RateLimited { retry_after },
            IntegrationError::QuotaExceeded { status } => AiError::QuotaExceeded {
                status: Some(status),
                retry_after: None,
            },
            IntegrationError::Offline { reason } => AiError::Offline { reason },
            IntegrationError::BadRequest { status, message } => {
                AiError::BadRequest { status, message }
            }
            IntegrationError::Provider { status, message } => AiError::Provider {
                status: Some(status),
                message,
            },
            IntegrationError::Unsupported { operation, .. } => {
                AiError::Unsupported { what: operation }
            }
        }
    }
}

impl Serialize for AiError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let status = self.status();
        let retry_after = self.retry_after();
        let len = 3 + usize::from(status.is_some()) + usize::from(retry_after.is_some());
        let mut s = serializer.serialize_struct("AiError", len)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_like_integration_error() {
        let e = AiError::RateLimited {
            retry_after: Some(Duration::from_secs(12)),
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({
                "key": "rate-limited",
                "message": "the provider is rate limiting requests",
                "retryable": true,
                "retryAfter": 12,
            })
        );
        let e = AiError::ModelNotFound {
            status: Some(404),
            message: "no llama9".into(),
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({
                "key": "model-not-found",
                "message": "the provider has no such model: no llama9",
                "retryable": false,
                "status": 404,
            })
        );
        assert_eq!(
            serde_json::to_value(AiError::Cancelled).unwrap(),
            json!({"key": "cancelled", "message": "the request was cancelled", "retryable": false})
        );
    }

    #[test]
    fn integration_errors_keep_their_key() {
        let cases = [
            IntegrationError::NotConfigured { what: "x" },
            IntegrationError::Auth { status: 401 },
            IntegrationError::RateLimited { retry_after: None },
            IntegrationError::QuotaExceeded { status: 402 },
            IntegrationError::Offline {
                reason: "timed out",
            },
            IntegrationError::BadRequest {
                status: None,
                message: "m".into(),
            },
            IntegrationError::Provider {
                status: 500,
                message: "m".into(),
            },
        ];
        for e in cases {
            assert_eq!(AiError::from(e.clone()).key(), e.key());
        }
    }
}
