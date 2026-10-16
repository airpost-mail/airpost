// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern-backend`: the self-hostable Tern server (docs/backend-plan.md).

pub mod auth;
pub mod backup;
pub mod cli;
pub mod config;
pub mod http;
pub mod ids;
pub mod repo;

/// The wire types live in `crates/ms-wire`, shared with the engine.
pub use ms_wire as wire;
