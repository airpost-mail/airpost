// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Yahoo and AOL (`TaskProcessor.cpp:129-216`, `SyncWorker.cpp:1212-1224, 1294-1298`,
//! `MCIMAPSession.cpp:857, 3705`), Copyright © 2017 Foundry 376, GPL-3.0.
//! mailcore2 (`MC*.cpp`) is Copyright © 2001-2013 MailCore team, BSD-3-Clause. See NOTICE.md
//! ("mailcore2").

use super::{greeting_has, host_is};
use crate::{ConnectionInfo, Provider, ServerProfile};

/// mailcore's `mYahooServer` (greeting contains `yahoo.com`), the hosts, or Yahoo's own
/// XYMHIGHESTMODSEQ capability.
pub fn detect(info: &ConnectionInfo) -> bool {
    greeting_has(info, "yahoo.com")
        || host_is(info, "yahoo.com")
        || host_is(info, "aol.com")
        || info.has_capability("XYMHIGHESTMODSEQ")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::Yahoo,
        // COPYUID's source and destination sets are not in the same order, which is why
        // `trust_copyuid_pairing` is off for every server.
        search_charset_null: true,
        xymhighestmodseq: true,
        ..ServerProfile::default()
    }
}
