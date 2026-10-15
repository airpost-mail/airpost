//! Courier-IMAP (`MCIMAPSession.cpp:862-867`, `TaskProcessor.cpp:2438`), Copyright © 2017
//! Foundry 376, GPL-3.0.

use super::greeting_has;
use crate::{ConnectionInfo, Provider, ServerProfile};

pub fn detect(info: &ConnectionInfo) -> bool {
    greeting_has(info, "Courier-IMAP")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::Courier,
        force_idle: true,
        force_namespace: true,
        // A message we APPEND is invisible to the selected folder until it is re-selected;
        // `reselect_after_append` is on for every server.
        ..ServerProfile::default()
    }
}
