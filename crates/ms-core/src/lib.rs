// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Foundations shared by every engine crate: errors, ids, constants, logging, the clock,
//! the account and identity, header and flag types, UID sets and the wake signals.
//!
//! Derived from Mailspring-Sync (`MailSync/SyncException.*`, `constants.h`, `MailUtils.*`,
//! `SPDLogExtensions.hpp`, `ThreadUtils.*`, `Models/Account.*`, `Models/Identity.*`),
//! Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.

pub mod account;
pub mod clock;
pub mod constants;
pub mod date;
pub mod error;
pub mod flags;
pub mod header;
pub mod ids;
pub mod log;
pub mod logmsg;
pub mod metadata;
pub mod protolog;
pub mod rfc2047;
pub mod tls_advice;
pub mod uid;
pub mod utf7;
pub mod wake;

pub use account::{Account, Identity, Security};
pub use clock::{Clock, ManualClock, SystemClock};
pub use error::{Error, ErrorCode, MailError, Result, SyncError};
pub use flags::{FolderFlags, MessageFlags};
pub use header::{Address, MessageHeader};
pub use metadata::Metadata;
pub use tls_advice::tls_failure_advice;
pub use uid::{UidRange, UidSet};
