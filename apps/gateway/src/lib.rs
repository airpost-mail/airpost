// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `airpost-gateway`: authenticated WebSocket and HTTP access to a Airpost engine for the web
//! client. It runs as the engine's user, serves the built web app
//! on the same origin, and authorises browsers with device tokens from a local pairing flow.

pub mod cli;
pub mod config;
pub mod control;
pub mod devices;
pub mod engine;
pub mod http;
