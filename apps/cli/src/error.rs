// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

use std::fmt;

use airpost_ipc::{code, ClientError};

/// What ends a command, and its exit code.
#[derive(Debug)]
pub enum CliError {
    /// 1: the command ran and failed.
    Failed(String),
    /// 2: the command line asks for something that can't be done as given.
    Usage(String),
    /// 3: no engine to talk to.
    NoEngine(String),
}

pub type Result<T> = std::result::Result<T, CliError>;

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::Failed(_) => 1,
            CliError::Usage(_) => 2,
            CliError::NoEngine(_) => 3,
        }
    }

    pub fn failed(msg: impl Into<String>) -> Self {
        CliError::Failed(msg.into())
    }

    pub fn usage(msg: impl Into<String>) -> Self {
        CliError::Usage(msg.into())
    }

    /// A failed call to the engine of `account`, with the engine's error data when it has
    /// any (a rejected task's `SyncError`).
    pub fn call(account: &str, method: &str, e: ClientError) -> Self {
        let mut msg = match &e {
            ClientError::Rpc(err) if err.code == code::NOT_FOUND => {
                format!("{account}: {}", err.message)
            }
            ClientError::Closed => format!(
                "{account}: the engine closed the connection during {method}; is Airpost quitting?"
            ),
            ClientError::Io(io) if e.is_timeout() => {
                format!("{account}: {method} timed out ({io}); the engine may be busy, try again")
            }
            _ => format!("{account}: {method} failed: {e}"),
        };
        if let ClientError::Rpc(err) = &e {
            if let Some(data) = &err.data {
                msg.push_str(&format!("\n{data}"));
            }
        }
        CliError::Failed(msg)
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Failed(m) | CliError::Usage(m) | CliError::NoEngine(m) => f.write_str(m),
        }
    }
}
