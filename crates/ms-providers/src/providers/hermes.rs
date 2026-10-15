//! Hermes (`MCIMAPSession.cpp:866, 4547`), Copyright © 2017 Foundry 376, GPL-3.0.

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
