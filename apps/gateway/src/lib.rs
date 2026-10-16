// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern-gateway`: authenticated WebSocket and HTTP access to a Tern engine for the web
//! client (docs/web-gateway-plan.md). It runs as the engine's user, serves the built web app
//! on the same origin, and authorises browsers with device tokens from a local pairing flow.

pub mod cli;
pub mod config;
pub mod control;
pub mod devices;
pub mod http;
