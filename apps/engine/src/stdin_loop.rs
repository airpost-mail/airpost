//! The main thread's command loop (`runListenOnMainThread`): one JSON object per line,
//! unparseable lines ignored. Once stdin has been closed for more than 30 s the parent is
//! gone and the process exits 141.

use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ms_core::constants::{
    thread, EXIT_STDIN_CLOSED, QUEUE_TASK_IDLE_DEBOUNCE_MS, STDIN_ORPHAN_TIMEOUT,
    STREAM_DELAY_MAIN_MS,
};
use ms_core::SyncError;
use serde_json::Value;

use crate::platform;
use ms_store::{Model, Task};
use ms_sync::WorkerContext;
use ms_tasks::TaskProcessor;

pub fn run(ctx: WorkerContext) -> i32 {
    let store = match ms_store::Store::open(&ctx.config_dir, ctx.deltas.clone()) {
        Ok(s) => s,
        Err(e) => {
            ms_core::critical!("Could not open the database: {e}");
            ms_core::log::flush();
            std::process::abort();
        }
    };
    store.set_stream_delay(STREAM_DELAY_MAIN_MS);
    let tasks = TaskProcessor::for_context(&ctx, &store);
    if let Err(e) = tasks.cleanup_tasks_after_launch() {
        abort_on_task_error(&e);
    }

    let fg_wake_queued = Arc::new(AtomicBool::new(false));
    let calendar_running = Arc::new(AtomicBool::new(false));
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
        handle(&ctx, &tasks, &packet, &fg_wake_queued, &calendar_running);
    }
}

/// stdin is closed (or broken) for good: the C++ polled `cin.good()` for 30 s before
/// deciding the parent was gone. A debugger attaching can interrupt stdin briefly; a
/// closed pipe does not come back, so this just waits out the same window.
fn wait_for_parent_then_exit() -> ! {
    let lost_at = Instant::now();
    while lost_at.elapsed() <= Duration::from_secs(STDIN_ORPHAN_TIMEOUT) {
        std::thread::sleep(Duration::from_millis(250));
    }
    platform::exit_now(EXIT_STDIN_CLOSED)
}

/// A database error from a task's local phase or a cancel leaves the task as it was; the
/// C++ main loop let it escape and aborted.
fn abort_on_task_error(e: &ms_core::Error) -> ! {
    ms_core::critical!("An unexpected error occurred during program execution: {e}");
    ms_core::log::flush();
    std::process::abort();
}

fn handle(
    ctx: &WorkerContext,
    tasks: &TaskProcessor<'_>,
    packet: &Value,
    fg_wake_queued: &Arc<AtomicBool>,
    calendar_running: &Arc<AtomicBool>,
) {
    match packet["type"].as_str().unwrap_or("") {
        "queue-task" => {
            let mut json = packet["task"].clone();
            if let Some(obj) = json.as_object_mut() {
                obj.insert("v".into(), Value::from(0));
                if let Ok(mut task) = Task::from_json(json) {
                    if let Err(e) = tasks.perform_local(&mut task) {
                        abort_on_task_error(&e);
                    }
                }
            }

            // Interrupt the foreground IDLE so it runs the remote phase, after a short
            // wait so tasks queued back to back go together instead of fighting the
            // foreground worker for the database.
            if fg_wake_queued
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                let flag = fg_wake_queued.clone();
                let fg = ctx.foreground.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(QUEUE_TASK_IDLE_DEBOUNCE_MS));
                    fg.interrupt();
                    flag.store(false, Ordering::SeqCst);
                });
            }
        }
        "cancel-task" => {
            // A task that has started (or finished) cannot always be dequeued, but a
            // deleted draft's pending saves can.
            if let Some(id) = packet["taskId"].as_str() {
                if let Err(e) = tasks.cancel(id) {
                    abort_on_task_error(&e);
                }
            }
        }
        "wake-workers" => {
            tracing::info!("{}", ms_core::logmsg::waking_all_workers());
            ctx.background_mark_all.store(true, Ordering::SeqCst);
            ms_core::wake::wake_all_workers();
            // The network may have changed while asleep; don't leave IDLE hanging on a
            // dead connection.
            ctx.foreground.interrupt();
        }
        "need-bodies" => {
            let ids = packet["ids"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            ctx.foreground.queue_bodies(ids);
            ctx.foreground.interrupt();
        }
        "sync-calendar" => {
            if calendar_running
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                let flag = calendar_running.clone();
                let c = ctx.clone();
                let spawned = std::thread::Builder::new()
                    .name(thread::CALENDAR.into())
                    .spawn(move || {
                        ms_sync::cal_contacts::run_calendar_pass(c);
                        flag.store(false, Ordering::SeqCst);
                    });
                if spawned.is_err() {
                    calendar_running.store(false, Ordering::SeqCst);
                }
            }
        }
        "test-crash" => {
            let e = SyncError::fatal("test", "triggered via cin");
            ms_core::critical!("{}", e.to_json());
            ms_core::log::flush();
            std::process::abort();
        }
        "test-segfault" => platform::raise_segfault(),
        _ => {}
    }
}
