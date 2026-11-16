// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! stdin commands (`runListenOnMainThread`'s reader): one JSON object per line,
//! unparseable lines ignored, each decoded into a [`Command`] for the main thread. Once
//! stdin has been closed for more than 30 s the parent is gone and the process exits 141.
//! With `--socket` stdin keeps both roles: it still takes the old commands, and it is the
//! lifeline.
//!
//! Derived from Mailspring-Sync `MailSync/main.cpp`, Copyright © 2017 Foundry 376, GPL-3.0.
//! See NOTICE.md.

use std::io::BufRead;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use airpost_core::constants::{thread, EXIT_STDIN_CLOSED, STDIN_ORPHAN_TIMEOUT};
use serde_json::Value;

use crate::dispatch::{Command, Request};
use crate::platform;

/// Starts the thread reading stdin into `commands`.
pub fn spawn_reader(commands: Sender<Command>) {
    let spawned = std::thread::Builder::new()
        .name(thread::STDIN.into())
        .spawn(move || read_commands(&commands));
    if let Err(e) = spawned {
        airpost_core::critical!("Could not start the {} thread: {e}", thread::STDIN);
        airpost_core::log::flush();
        std::process::abort();
    }
}

fn read_commands(commands: &Sender<Command>) -> ! {
    let stdin = std::io::stdin();
    let mut line = Vec::new();
    loop {
        line.clear();
        // Raw bytes, as getline reads them: a line that is not UTF-8 is dropped like any
        // other unparseable line, not taken for a closed stdin.
        match stdin.lock().read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => wait_for_parent_then_exit(),
            Ok(_) => {}
        }
        let Ok(text) = std::str::from_utf8(&line) else {
            continue;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(packet) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if let Some(request) = decode(&packet) {
            let _ = commands.send(Command {
                request,
                reply: None,
            });
        }
    }
}

/// stdin is closed (or broken) for good: the C++ polled `cin.good()` for 30 s before
/// deciding the parent was gone. A debugger attaching can interrupt stdin briefly; a
/// closed pipe does not come back, so this just waits out the same window.
pub(crate) fn wait_for_parent_then_exit() -> ! {
    let lost_at = Instant::now();
    while lost_at.elapsed() <= Duration::from_secs(STDIN_ORPHAN_TIMEOUT) {
        std::thread::sleep(Duration::from_millis(250));
    }
    platform::exit_now(EXIT_STDIN_CLOSED)
}

/// The request a stdin packet names; `None` for unknown types and a `cancel-task` without
/// a `taskId`, which do nothing.
fn decode(packet: &Value) -> Option<Request> {
    Some(match packet["type"].as_str().unwrap_or("") {
        "queue-task" => Request::QueueTask(packet["task"].clone()),
        "cancel-task" => Request::CancelTask(packet["taskId"].as_str()?.to_owned()),
        "wake-workers" => Request::WakeWorkers,
        "need-bodies" => Request::NeedBodies(
            packet["ids"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
        ),
        "sync-calendar" => Request::SyncCalendar,
        "test-crash" => Request::TestCrash,
        "test-segfault" => Request::TestSegfault,
        _ => return None,
    })
}
