// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `airpost-backend`: the self-hostable Airpost server (docs/backend-plan.md).

pub mod auth;
pub mod backup;
pub mod cli;
pub mod config;
pub mod http;
pub mod ids;
pub mod repo;

/// The wire types live in `crates/airpost-wire`, shared with the engine.
pub use airpost_wire as wire;
