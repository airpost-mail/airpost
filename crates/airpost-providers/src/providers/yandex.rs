// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Yandex (`MCIMAPSession.cpp:4505`, `SyncWorker.cpp:310-313`), Copyright © 2017 Foundry 376,
//! GPL-3.0.
//! mailcore2 (`MC*.cpp`) is Copyright © 2001-2013 MailCore team, BSD-3-Clause. See NOTICE.md
//! ("mailcore2").

use super::greeting_has;
use crate::{ConnectionInfo, Provider, ServerProfile};

pub fn detect(info: &ConnectionInfo) -> bool {
    greeting_has(info, "Yandex IMAP4rev1")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::Yandex,
        // Advertised, but the connection is dropped when it is used.
        xlist_disabled: true,
        // Drops IDLE connections at random; `tolerate_idle_disconnect` is on for every server.
        ..ServerProfile::default()
    }
}
