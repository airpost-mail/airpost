//! home.pl's IdeaImapServer (`MCIMAPSession.cpp:4502-4504`, `SyncWorker.cpp:447-460`),
//! Copyright © 2017 Foundry 376, GPL-3.0.

use super::greeting_has;
use crate::{ConnectionInfo, Provider, ServerProfile};

pub fn detect(info: &ConnectionInfo) -> bool {
    greeting_has(info, "IdeaImapServer")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::HomePl,
        // XLIST is advertised but its responses are malformed.
        xlist_disabled: true,
        // STATUS answers UIDNEXT 0 (Geary imap-status-data.vala).
        status_uidnext_unreliable: true,
        ..ServerProfile::default()
    }
}
