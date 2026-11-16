// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! DavMail, the IMAP gateway to Exchange (`SyncWorker.cpp:811-834`), Copyright © 2017
//! Foundry 376, GPL-3.0.

use super::greeting_has;
use crate::{ConnectionInfo, Provider, ServerProfile};

pub fn detect(info: &ConnectionInfo) -> bool {
    greeting_has(info, "DavMail")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::DavMail,
        merge_duplicate_list_entries: true,
        ..ServerProfile::default()
    }
}
