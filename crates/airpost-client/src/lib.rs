// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The client side of the engine protocol,
//! shared by `airpost`, `airpost-mcp` and later the gateway:
//!
//! - [`discovery`]: the profile directory and the `run/*.json` files engines write;
//! - [`Engine`] and [`Engines`]: connected engines after `hello`, and the router that sends an
//!   account's calls to the engine holding it;
//! - [`tasks`]: the task JSON the desktop queues, and the checks for a finished task;
//! - [`metadata`]: the values the desktop's send-later, reminder and tracking plugins write;
//! - [`search`]: the zone and clock a client sends with grammar searches;
//! - [`utf7`]: modified UTF-7 folder and label names;
//! - [`files`]: attachment file names as the desktop saves them.
//!
//! The wire types and the blocking client are `airpost-ipc`'s.

pub mod discovery;
mod engines;
pub mod files;
pub mod metadata;
mod platform;
pub mod search;
pub mod tasks;
pub mod utf7;

pub use engines::{AccountRef, Engine, EngineError, Engines, CALL_TIMEOUT, HELLO_TIMEOUT};
pub use platform::pid_alive;
