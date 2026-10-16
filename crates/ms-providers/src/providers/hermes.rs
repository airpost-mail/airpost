// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Hermes (`MCIMAPSession.cpp:866, 4547`), Copyright © 2017 Foundry 376, GPL-3.0.
//! mailcore2 (`MC*.cpp`) is Copyright © 2001-2013 MailCore team, BSD-3-Clause. See NOTICE.md
//! ("mailcore2").

use super::greeting_has;
use crate::{ConnectionInfo, Provider, ServerProfile};

/// mailcore's `mHermesServer`.
pub fn detect(info: &ConnectionInfo) -> bool {
    greeting_has(info, "Hermes")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::Hermes,
        // NAMESPACE is advertised but not implemented properly.
        ignore_namespace_capability: true,
        ..ServerProfile::default()
    }
}
