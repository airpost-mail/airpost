// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The client side of the engine protocol (docs/engine-protocol-plan.md, docs/desktop-client-plan.md
//! T20), shared by `tern`, `tern-mcp` and later the gateway:
//!
//! - [`discovery`]: the profile directory and the `run/*.json` files engines write;
//! - [`Engine`] and [`Engines`]: connected engines after `hello`, and the router that sends an
//!   account's calls to the engine holding it;
//! - [`tasks`]: the task JSON the desktop queues, and the checks for a finished task.
//!
//! The wire types and the blocking client are `ms-ipc`'s.

pub mod discovery;
mod engines;
mod platform;
pub mod tasks;

pub use engines::{AccountRef, Engine, EngineError, Engines, CALL_TIMEOUT, HELLO_TIMEOUT};
pub use platform::pid_alive;
