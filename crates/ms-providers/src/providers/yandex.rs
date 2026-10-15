//! Yandex (`MCIMAPSession.cpp:4505`, `SyncWorker.cpp:310-313`), Copyright © 2017 Foundry 376,
//! GPL-3.0.

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
