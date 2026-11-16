// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! iCloud (`MailUtils.cpp:877-884`, `SyncWorker.cpp:402`, `Account.cpp:125`), Copyright © 2017
//! Foundry 376, GPL-3.0.

use crate::{ConnectionInfo, Provider, ServerProfile};

/// `Account::isICloud`: the host contains `imap.mail.me.com`.
pub fn detect(info: &ConnectionInfo) -> bool {
    info.host.contains("imap.mail.me.com")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::ICloud,
        // iCloud's QRESYNC returns malformed VANISHED responses and no ENABLED response,
        // which made messages look deleted (developer.apple.com/forums/thread/694251).
        disable_qresync: true,
        ..ServerProfile::default()
    }
}
