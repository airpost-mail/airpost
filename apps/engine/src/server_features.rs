// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! What the connected Tern server offers, as its last `/tern/v1/me` answer said: the server
//! half of `capabilities` (`readReceipts`, `linkTracking`). Every account's HTTP metadata
//! backend reports here; they all talk to the same server.

use std::sync::Mutex;

use ms_net::Me;

/// `None`: no server answered, or its token was refused. `Some(tracking)` otherwise.
static TRACKING: Mutex<Option<bool>> = Mutex::new(None);

/// A backend's `/tern/v1/me` answer, or `None` once the server is gone or refuses the token.
/// A change is announced as `capabilities.changed`.
pub fn reported(me: Option<&Me>) {
    let next = me.map(|m| m.features.tracking);
    let changed = {
        let mut current = TRACKING.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::replace(&mut *current, next) != next
    };
    if changed {
        #[cfg(unix)]
        crate::ai_triage::capabilities_changed();
    }
}

/// Whether the connected server offers tracking; `None` without one.
pub fn tracking() -> Option<bool> {
    *TRACKING.lock().unwrap_or_else(|e| e.into_inner())
}
