// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! hMailServer (`SyncWorker.cpp:447-460`), Copyright © 2017 Foundry 376, GPL-3.0.

use super::greeting_has;
use crate::{ConnectionInfo, Provider, ServerProfile};

pub fn detect(info: &ConnectionInfo) -> bool {
    greeting_has(info, "hMailServer")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::HMailServer,
        // STATUS answers UIDNEXT 0 (Geary imap-status-data.vala).
        status_uidnext_unreliable: true,
        ..ServerProfile::default()
    }
}
