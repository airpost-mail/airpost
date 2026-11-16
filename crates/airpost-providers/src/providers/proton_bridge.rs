// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! ProtonMail Bridge (`SyncWorker.cpp:498-507, 811-834`), Copyright © 2017 Foundry 376, GPL-3.0.

use super::greeting_has;
use crate::{ConnectionInfo, Provider, ServerProfile};

pub fn detect(info: &ConnectionInfo) -> bool {
    greeting_has(info, "Proton Mail Bridge")
        || greeting_has(info, "ProtonMail Bridge")
        || info
            .id_response
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("name") && v.contains("Bridge"))
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::ProtonBridge,
        skip_all_mail_folder: true,
        merge_duplicate_list_entries: true,
        ..ServerProfile::default()
    }
}
