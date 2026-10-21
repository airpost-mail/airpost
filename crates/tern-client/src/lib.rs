// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The client side of the engine protocol,
//! shared by `tern`, `tern-mcp` and later the gateway:
//!
//! - [`discovery`]: the profile directory and the `run/*.json` files engines write;
//! - [`Engine`] and [`Engines`]: connected engines after `hello`, and the router that sends an
//!   account's calls to the engine holding it;
//! - [`tasks`]: the task JSON the desktop queues, and the checks for a finished task;
//! - [`search`]: the zone and clock a client sends with grammar searches.
//!
//! The wire types and the blocking client are `ms-ipc`'s.

pub mod discovery;
mod engines;
mod platform;
pub mod search;
pub mod tasks;

pub use engines::{AccountRef, Engine, EngineError, Engines, CALL_TIMEOUT, HELLO_TIMEOUT};
pub use platform::pid_alive;
