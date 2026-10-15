//! Zoho Mail (`SyncWorker.cpp:811-834`), Copyright © 2017 Foundry 376, GPL-3.0.

use super::{greeting_has, host_is};
use crate::{ConnectionInfo, Provider, ServerProfile};

pub fn detect(info: &ConnectionInfo) -> bool {
    [
        "zoho.com",
        "zoho.eu",
        "zoho.in",
        "zoho.com.au",
        "zoho.jp",
        "zohomail.com",
    ]
    .iter()
    .any(|d| host_is(info, d))
        || greeting_has(info, "Zoho")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::Zoho,
        merge_duplicate_list_entries: true,
        ..ServerProfile::default()
    }
}
