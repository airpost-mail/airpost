// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Error types shared by every engine crate.
//!
//! Derived from Mailspring-Sync `MailSync/SyncException.cpp`, `MailSync/GenericException.cpp`
//! and the `ErrorCodeToTypeMap` in `MailSync/constants.h`, Copyright © 2017 Foundry 376,
//! GPL-3.0, and mailcore2's `ErrorCode` (`MCMessageConstants.h`), Copyright © 2001-2013
//! MailCore team, BSD-3-Clause. See NOTICE.md ("mailcore2").
//!
//! - [`SyncError`] is the C++ `SyncException`: a logical failure with a key the client
//!   understands, and the retry policy (`retryable`, `offline`) the workers act on.
//! - [`MailError`] is what a protocol session (IMAP, SMTP) returns: a mailcore-style
//!   [`ErrorCode`] the caller can match on, plus server detail for the log. It becomes a
//!   `SyncError` through [`MailError::into_sync`], which names the failing call the way
//!   `throw SyncException(err, "<call>")` did.
//! - [`Error`] is the crate-wide error. A task that fails with `Error::Database` is left as
//!   it was (TaskProcessor keeps SQLite errors out of the task's error JSON); every other
//!   variant ends the task with [`Error::to_json`].

use serde_json::{json, Value};
use std::fmt;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// mailcore2's `ErrorCode` (`MCMessageConstants.h`). The discriminants match mailcore's, so
/// "Idle exited with code {}" logs the same numbers; [`ErrorCode::name`] gives the strings
/// of `ErrorCodeToTypeMap` (`constants.h:446`), which the client keys on in `test` mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ErrorCode {
    None = 0,
    Connection,
    TLSNotAvailable,
    Parse,
    Certificate,
    Authentication,
    GmailIMAPNotEnabled,
    GmailExceededBandwidthLimit,
    GmailTooManySimultaneousConnections,
    MobileMeMoved,
    YahooUnavailable,
    NonExistantFolder,
    Rename,
    Delete,
    Create,
    Subscribe,
    Append,
    Copy,
    Expunge,
    Fetch,
    Idle,
    Identity,
    Namespace,
    Store,
    Capability,
    StartTLSNotAvailable,
    SendMessageIllegalAttachment,
    StorageLimit,
    SendMessageNotAllowed,
    NeedsConnectToWebmail,
    SendMessage,
    AuthenticationRequired,
    FetchMessageList,
    DeleteMessage,
    InvalidAccount,
    File,
    Compression,
    NoSender,
    NoRecipient,
    Noop,
    GmailApplicationSpecificPasswordRequired,
    ServerDate,
    NoValidServerFound,
    CustomCommand,
    YahooSendMessageSpamSuspected,
    YahooSendMessageDailyLimitExceeded,
    OutlookLoginViaWebBrowser,
    TiscaliSimplePassword,
    InvalidRelaySMTP,
    NoImplementedAuthMethods,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 50] = [
        ErrorCode::None,
        ErrorCode::Connection,
        ErrorCode::TLSNotAvailable,
        ErrorCode::Parse,
        ErrorCode::Certificate,
        ErrorCode::Authentication,
        ErrorCode::GmailIMAPNotEnabled,
        ErrorCode::GmailExceededBandwidthLimit,
        ErrorCode::GmailTooManySimultaneousConnections,
        ErrorCode::MobileMeMoved,
        ErrorCode::YahooUnavailable,
        ErrorCode::NonExistantFolder,
        ErrorCode::Rename,
        ErrorCode::Delete,
        ErrorCode::Create,
        ErrorCode::Subscribe,
        ErrorCode::Append,
        ErrorCode::Copy,
        ErrorCode::Expunge,
        ErrorCode::Fetch,
        ErrorCode::Idle,
        ErrorCode::Identity,
        ErrorCode::Namespace,
        ErrorCode::Store,
        ErrorCode::Capability,
        ErrorCode::StartTLSNotAvailable,
        ErrorCode::SendMessageIllegalAttachment,
        ErrorCode::StorageLimit,
        ErrorCode::SendMessageNotAllowed,
        ErrorCode::NeedsConnectToWebmail,
        ErrorCode::SendMessage,
        ErrorCode::AuthenticationRequired,
        ErrorCode::FetchMessageList,
        ErrorCode::DeleteMessage,
        ErrorCode::InvalidAccount,
        ErrorCode::File,
        ErrorCode::Compression,
        ErrorCode::NoSender,
        ErrorCode::NoRecipient,
        ErrorCode::Noop,
        ErrorCode::GmailApplicationSpecificPasswordRequired,
        ErrorCode::ServerDate,
        ErrorCode::NoValidServerFound,
        ErrorCode::CustomCommand,
        ErrorCode::YahooSendMessageSpamSuspected,
        ErrorCode::YahooSendMessageDailyLimitExceeded,
        ErrorCode::OutlookLoginViaWebBrowser,
        ErrorCode::TiscaliSimplePassword,
        ErrorCode::InvalidRelaySMTP,
        ErrorCode::NoImplementedAuthMethods,
    ];

    /// The `ErrorCodeToTypeMap` name, e.g. `"ErrorConnection"`.
    pub fn name(self) -> &'static str {
        match self {
            ErrorCode::None => "ErrorNone",
            ErrorCode::Connection => "ErrorConnection",
            ErrorCode::TLSNotAvailable => "ErrorTLSNotAvailable",
            ErrorCode::Parse => "ErrorParse",
            ErrorCode::Certificate => "ErrorCertificate",
            ErrorCode::Authentication => "ErrorAuthentication",
            ErrorCode::GmailIMAPNotEnabled => "ErrorGmailIMAPNotEnabled",
            ErrorCode::GmailExceededBandwidthLimit => "ErrorGmailExceededBandwidthLimit",
            ErrorCode::GmailTooManySimultaneousConnections => {
                "ErrorGmailTooManySimultaneousConnections"
            }
            ErrorCode::MobileMeMoved => "ErrorMobileMeMoved",
            ErrorCode::YahooUnavailable => "ErrorYahooUnavailable",
            ErrorCode::NonExistantFolder => "ErrorNonExistantFolder",
            ErrorCode::Rename => "ErrorRename",
            ErrorCode::Delete => "ErrorDelete",
            ErrorCode::Create => "ErrorCreate",
            ErrorCode::Subscribe => "ErrorSubscribe",
            ErrorCode::Append => "ErrorAppend",
            ErrorCode::Copy => "ErrorCopy",
            ErrorCode::Expunge => "ErrorExpunge",
            ErrorCode::Fetch => "ErrorFetch",
            ErrorCode::Idle => "ErrorIdle",
            ErrorCode::Identity => "ErrorIdentity",
            ErrorCode::Namespace => "ErrorNamespace",
            ErrorCode::Store => "ErrorStore",
            ErrorCode::Capability => "ErrorCapability",
            ErrorCode::StartTLSNotAvailable => "ErrorStartTLSNotAvailable",
            ErrorCode::SendMessageIllegalAttachment => "ErrorSendMessageIllegalAttachment",
            ErrorCode::StorageLimit => "ErrorStorageLimit",
            ErrorCode::SendMessageNotAllowed => "ErrorSendMessageNotAllowed",
            ErrorCode::NeedsConnectToWebmail => "ErrorNeedsConnectToWebmail",
            ErrorCode::SendMessage => "ErrorSendMessage",
            ErrorCode::AuthenticationRequired => "ErrorAuthenticationRequired",
            ErrorCode::FetchMessageList => "ErrorFetchMessageList",
            ErrorCode::DeleteMessage => "ErrorDeleteMessage",
            ErrorCode::InvalidAccount => "ErrorInvalidAccount",
            ErrorCode::File => "ErrorFile",
            ErrorCode::Compression => "ErrorCompression",
            ErrorCode::NoSender => "ErrorNoSender",
            ErrorCode::NoRecipient => "ErrorNoRecipient",
            ErrorCode::Noop => "ErrorNoop",
            ErrorCode::GmailApplicationSpecificPasswordRequired => {
                "ErrorGmailApplicationSpecificPasswordRequired"
            }
            ErrorCode::ServerDate => "ErrorServerDate",
            ErrorCode::NoValidServerFound => "ErrorNoValidServerFound",
            ErrorCode::CustomCommand => "ErrorCustomCommand",
            ErrorCode::YahooSendMessageSpamSuspected => "ErrorYahooSendMessageSpamSuspected",
            ErrorCode::YahooSendMessageDailyLimitExceeded => {
                "ErrorYahooSendMessageDailyLimitExceeded"
            }
            ErrorCode::OutlookLoginViaWebBrowser => "ErrorOutlookLoginViaWebBrowser",
            ErrorCode::TiscaliSimplePassword => "ErrorTiscaliSimplePassword",
            ErrorCode::InvalidRelaySMTP => "ErrorInvalidRelaySMTP",
            ErrorCode::NoImplementedAuthMethods => "ErrorNoImplementedAuthMethods",
        }
    }

    pub fn as_i32(self) -> i32 {
        self as u8 as i32
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// `std::exception::what()` for a `SyncException`, which never overrides it. The key and
/// debuginfo carry the information; `what` is kept only so the JSON shape is unchanged.
pub const SYNC_ERROR_WHAT: &str = "std::exception";

/// The C++ `SyncException`. `to_json` produces exactly
/// `{"debuginfo","key","offline","retryable","what"}` (`SyncException.cpp:73-81`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncError {
    pub key: String,
    pub debuginfo: String,
    pub retryable: bool,
    pub offline: bool,
}

impl SyncError {
    pub fn new(
        key: impl Into<String>,
        debuginfo: impl Into<String>,
        retryable: bool,
        offline: bool,
    ) -> Self {
        SyncError {
            key: key.into(),
            debuginfo: debuginfo.into(),
            retryable,
            offline,
        }
    }

    /// A non-retryable logical failure, `SyncException(key, di, false)`.
    pub fn fatal(key: impl Into<String>, debuginfo: impl Into<String>) -> Self {
        SyncError::new(key, debuginfo, false, false)
    }

    /// A network failure the worker sleeps through and the client shows as offline. This is
    /// what `SyncException(CURLcode, di)` produced for resolve, connect, timeout, TLS and
    /// transfer errors.
    pub fn offline(key: impl Into<String>, debuginfo: impl Into<String>) -> Self {
        SyncError::new(key, debuginfo, true, true)
    }

    /// `SyncException(ErrorCode, di)`: connection errors are retryable and offline; parse and
    /// fetch errors are retryable (they usually follow an abrupt disconnect).
    pub fn from_code(code: ErrorCode, debuginfo: impl Into<String>) -> Self {
        let (retryable, offline) = match code {
            ErrorCode::Connection => (true, true),
            ErrorCode::Parse | ErrorCode::Fetch => (true, false),
            _ => (false, false),
        };
        // Note: ErrorNone has a name in the map, so the key is never empty here.
        SyncError::new(code.name(), debuginfo, retryable, offline)
    }

    pub fn is_retryable(&self) -> bool {
        self.retryable
    }

    pub fn is_offline(&self) -> bool {
        self.offline
    }

    pub fn to_json(&self) -> Value {
        json!({
            "what": SYNC_ERROR_WHAT,
            "key": self.key,
            "debuginfo": self.debuginfo,
            "retryable": self.retryable,
            "offline": self.offline,
        })
    }
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.key, self.debuginfo)
    }
}

impl std::error::Error for SyncError {}

/// A protocol session failure: the mailcore error code plus whatever the server or the
/// transport said, which goes to the log but not into the task's error JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailError {
    pub code: ErrorCode,
    pub detail: String,
}

impl MailError {
    pub fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        MailError {
            code,
            detail: detail.into(),
        }
    }

    /// `throw SyncException(err, "<call>")`: the debuginfo names the failing call.
    pub fn into_sync(self, call: &str) -> SyncError {
        SyncError::from_code(self.code, call)
    }
}

impl fmt::Display for MailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.detail.is_empty() {
            f.write_str(self.code.name())
        } else {
            write!(f, "{} ({})", self.code.name(), self.detail)
        }
    }
}

impl std::error::Error for MailError {}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Sync(#[from] SyncError),
    #[error("{0}")]
    Database(#[from] rusqlite::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// An unclassified failure, the equivalent of a `std::runtime_error`.
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }

    pub fn as_sync(&self) -> Option<&SyncError> {
        match self {
            Error::Sync(e) => Some(e),
            _ => None,
        }
    }

    /// Workers sleep and retry on these; anything else aborts the process.
    pub fn is_retryable(&self) -> bool {
        self.as_sync().is_some_and(SyncError::is_retryable)
    }

    /// What a worker loop sleeps through and retries rather than stopping the account
    /// for (TD44): a retryable `SyncError`, a database that is busy, locked, full or failed
    /// an I/O call, and I/O errors. A corrupt database or a schema error stays fatal.
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Sync(e) => e.is_retryable(),
            Error::Database(rusqlite::Error::SqliteFailure(e, _)) => matches!(
                e.code,
                rusqlite::ErrorCode::DatabaseBusy
                    | rusqlite::ErrorCode::DatabaseLocked
                    | rusqlite::ErrorCode::SystemIoFailure
                    | rusqlite::ErrorCode::DiskFull
            ),
            Error::Io(_) => true,
            _ => false,
        }
    }

    pub fn is_offline(&self) -> bool {
        self.as_sync().is_some_and(SyncError::is_offline)
    }

    /// SQLITE_CONSTRAINT (19), the insert race the C++ caught with `getErrorCode() != 19`.
    pub fn is_constraint_violation(&self) -> bool {
        matches!(
            self,
            Error::Database(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation
        )
    }

    /// The JSON stored in a failed task's `error`. A `SyncError` keeps its own shape; any
    /// other failure is reported as `unhandled-exception` (`TaskProcessor.cpp:697-715`).
    pub fn to_json(&self) -> Value {
        match self {
            Error::Sync(e) => e.to_json(),
            other => unexpected_error_json(&other.to_string()),
        }
    }
}

impl From<MailError> for Error {
    fn from(e: MailError) -> Self {
        let detail = e.detail.clone();
        Error::Sync(SyncError::from_code(e.code, detail))
    }
}

/// `errorJSONForUnexpectedException`: the message is scrubbed to printable ASCII (it ends up
/// in a JSON column) and cut to 1024 bytes.
pub fn unexpected_error_json(what: &str) -> Value {
    let mut safe: String = what
        .bytes()
        .map(|b| {
            if (0x20..=0x7e).contains(&b) {
                b as char
            } else {
                '?'
            }
        })
        .collect();
    safe.truncate(1024);
    json!({
        "what": safe,
        "key": "unhandled-exception",
        "debuginfo": safe,
        "retryable": false,
        "offline": false,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn busy_full_and_io_errors_are_transient() {
        let sqlite = |code: i32| {
            Error::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                None,
            ))
        };
        assert!(sqlite(rusqlite::ffi::SQLITE_BUSY).is_transient());
        assert!(sqlite(rusqlite::ffi::SQLITE_LOCKED).is_transient());
        assert!(sqlite(rusqlite::ffi::SQLITE_FULL).is_transient());
        assert!(sqlite(rusqlite::ffi::SQLITE_IOERR).is_transient());
        assert!(!sqlite(rusqlite::ffi::SQLITE_CORRUPT).is_transient());
        assert!(!sqlite(rusqlite::ffi::SQLITE_ERROR).is_transient());
        assert!(Error::Io(std::io::Error::other("x")).is_transient());
        assert!(Error::from(SyncError::offline("ErrorConnection", "x")).is_transient());
        assert!(!Error::from(SyncError::fatal("k", "x")).is_transient());
        assert!(!Error::other("x").is_transient());
    }

    use super::*;

    #[test]
    fn sync_error_json_shape() {
        let e = SyncError::new("send-failed", "ErrorSendMessage", false, false);
        assert_eq!(
            e.to_json().to_string(),
            r#"{"debuginfo":"ErrorSendMessage","key":"send-failed","offline":false,"retryable":false,"what":"std::exception"}"#
        );
    }

    #[test]
    fn code_policy() {
        let c = SyncError::from_code(ErrorCode::Connection, "connectIfNeeded");
        assert_eq!(
            (c.key.as_str(), c.retryable, c.offline),
            ("ErrorConnection", true, true)
        );
        let p = SyncError::from_code(ErrorCode::Parse, "x");
        assert_eq!((p.retryable, p.offline), (true, false));
        let f = SyncError::from_code(ErrorCode::Fetch, "x");
        assert_eq!((f.retryable, f.offline), (true, false));
        let a = SyncError::from_code(ErrorCode::Authentication, "x");
        assert_eq!((a.retryable, a.offline), (false, false));
    }

    #[test]
    fn names_follow_discriminants() {
        for (i, code) in ErrorCode::ALL.iter().enumerate() {
            assert_eq!(code.as_i32(), i as i32);
            assert!(code.name().starts_with("Error"));
        }
        assert_eq!(ErrorCode::Idle.as_i32(), 20);
        assert_eq!(ErrorCode::SendMessage.as_i32(), 30);
        assert_eq!(
            ErrorCode::GmailApplicationSpecificPasswordRequired.as_i32(),
            40
        );
    }

    #[test]
    fn unexpected_is_scrubbed() {
        let long = format!("bad\u{e9}{}", "x".repeat(2000));
        let v = unexpected_error_json(&long);
        let what = v["what"].as_str().unwrap();
        assert_eq!(what.len(), 1024);
        assert!(what.starts_with("bad??x"));
        assert_eq!(v["key"], "unhandled-exception");
    }
}
