//! Dovecot (`SyncWorker.cpp:1184-1194, 1348-1361`, `MCIMAPSession.cpp:1467`), Copyright © 2017
//! Foundry 376, GPL-3.0.

use super::greeting_has;
use crate::{ConnectionInfo, Provider, ServerProfile};

pub fn detect(info: &ConnectionInfo) -> bool {
    greeting_has(info, "Dovecot")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::Dovecot,
        // STATUS of the selected folder answers from the connection's stale view.
        noop_before_selected_status: true,
        // UIDVALIDITY changing under a selected folder ends the connection with BYE.
        bye_on_uidvalidity_change: true,
        ..ServerProfile::default()
    }
}
