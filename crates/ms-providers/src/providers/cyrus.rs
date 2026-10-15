//! Cyrus IMAP and Fastmail (`SyncWorker.cpp:1394-1400`), Copyright © 2017 Foundry 376, GPL-3.0.

use super::{greeting_has, host_is};
use crate::{ConnectionInfo, Provider, ServerProfile};

pub fn detect(info: &ConnectionInfo) -> bool {
    greeting_has(info, "Cyrus IMAP") || host_is(info, "fastmail.com")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::Cyrus,
        // VANISHED is limited to UIDs in the requested set and `*` is the highest UID still
        // present, so expunges above it are never reported (cyrus-imapd #6071).
        vanished_upper_bound_uidnext: true,
        ..ServerProfile::default()
    }
}
