// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Any server no other provider file matches.

use crate::ServerProfile;

pub fn profile() -> ServerProfile {
    ServerProfile::default()
}
