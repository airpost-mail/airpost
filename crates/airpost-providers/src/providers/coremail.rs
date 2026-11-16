// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Coremail / NetEase (`MailUtils.cpp:849-856`, `MCIMAPSession.cpp:857-861, 4819`, libetpan
//! `mailimap.c:2461, 2550`, `SyncWorker.cpp:447-460`), Copyright © 2017 Foundry 376, GPL-3.0.
//! mailcore2 (`MCIMAPSession.cpp`) is Copyright © 2001-2013 MailCore team and libetpan
//! (`mailimap.c`) Copyright © 2001-2005 DINH Viet Hoa, both BSD-3-Clause. See NOTICE.md
//! ("mailcore2", "libEtPan!").

use super::greeting_has;
use crate::{ConnectionInfo, Provider, ServerProfile};

/// mailcore's `mCoremailServer`.
pub fn detect(info: &ConnectionInfo) -> bool {
    greeting_has(info, "Coremail System IMap Server Ready")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::Coremail,
        // SELECT is refused with "Unsafe Login" until the client has sent ID (RFC 2971).
        id_before_select: true,
        c_prefixed_tags: true,
        // STATUS omits UIDNEXT.
        status_uidnext_unreliable: true,
        ..ServerProfile::default()
    }
}
