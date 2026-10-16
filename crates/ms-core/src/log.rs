// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The engine log: a `tracing` subscriber that reproduces the C++ engine's spdlog output.
//! Derived from Mailspring-Sync `MailSync/SPDLogExtensions.hpp` and the logger setup in
//! `main.cpp`, Copyright © 2017 Foundry 376, GPL-3.0 (see NOTICE.md), and spdlog's
//! `rotating_file_sink`.
//!
//! - `sync` mode without `--orphan` writes `$CONFIG_DIR_PATH/mailsync-<accountId>.log`:
//!   `<pid> [YYYY-mm-dd HH:MM:SS.mmm] [<thread>] [<level>] <message>`, local time. At
//!   5 MB the file rotates to `.log.1`, `.log.1` to `.log.2`, up to `.log.3`. Lines are
//!   buffered and flushed about 1 s after activity; the client reads the last 3000 bytes
//!   for crash reports and the harness tails the file every 0.1 s.
//! - Every other mode logs `<level>: <message>` to stdout.
//! - `critical` lines also go to stderr, which the client turns into the process error,
//!   and flush the file at once.
//!
//! Levels are `info`, `warning`, `error` and `critical`. tracing has no critical level: use
//! [`critical!`](crate::critical), which logs at ERROR with a `critical` field. The thread
//! name is the std thread's name, so worker threads must be spawned with
//! `std::thread::Builder::name` (names in [`crate::constants::thread`]).
//!
//! Only events from the engine's own crates (targets `ms_*` and `mailsync`) at INFO and
//! above are recorded; dependencies' tracing output is dropped.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

use crate::constants::{LOG_FLUSH_DEBOUNCE_MS, LOG_MAX_BYTES, LOG_MAX_FILES};

const EOL: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// Logs at the `critical` level: ERROR in tracing terms, printed as `[critical]`, copied to
/// stderr and flushed immediately. Use it only for what ends the process.
#[macro_export]
macro_rules! critical {
    ($($arg:tt)+) => {
        ::tracing::error!(critical = true, $($arg)+)
    };
}

/// `$CONFIG_DIR_PATH/mailsync-<accountId>.log`.
pub fn log_path(config_dir: &Path, account_id: &str) -> PathBuf {
    config_dir.join(format!("mailsync-{account_id}.log"))
}

/// The name spdlog gives rotation `index` of `base`: `base`, `base.1`, `base.2`, ...
pub fn rotated_path(base: &Path, index: usize) -> PathBuf {
    if index == 0 {
        base.to_path_buf()
    } else {
        let mut s = base.as_os_str().to_owned();
        s.push(format!(".{index}"));
        PathBuf::from(s)
    }
}

/// A size-rotating log file with spdlog's semantics: the size counts the existing file, a
/// line that would take it past the limit rotates first, and the line lands in the new file.
pub struct RotatingFile {
    base: PathBuf,
    max_bytes: u64,
    max_files: usize,
    size: u64,
    out: BufWriter<File>,
}

impl RotatingFile {
    pub fn open(base: &Path, max_bytes: u64, max_files: usize) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(base)?;
        let size = file.metadata()?.len();
        Ok(RotatingFile {
            base: base.to_path_buf(),
            max_bytes,
            max_files,
            size,
            out: BufWriter::with_capacity(64 * 1024, file),
        })
    }

    pub fn write_line(&mut self, line: &str) -> io::Result<()> {
        self.size += line.len() as u64;
        if self.size > self.max_bytes {
            self.rotate()?;
            self.size = line.len() as u64;
        }
        self.out.write_all(line.as_bytes())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.out.flush()?;
        for i in (1..=self.max_files).rev() {
            let src = rotated_path(&self.base, i - 1);
            let target = rotated_path(&self.base, i);
            if target.exists() {
                fs::remove_file(&target)?;
            }
            if src.exists() {
                fs::rename(&src, &target)?;
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.base)?;
        self.out = BufWriter::with_capacity(64 * 1024, file);
        Ok(())
    }
}

struct FileSink {
    file: Mutex<RotatingFile>,
    pending: Mutex<FlushState>,
    cv: Condvar,
}

#[derive(Default)]
struct FlushState {
    unflushed: usize,
    exit: bool,
}

impl FileSink {
    fn write(&self, line: &str) {
        if let Ok(mut f) = self.file.lock() {
            // A full disk must not take the engine down; the line is lost.
            let _ = f.write_line(line);
        }
        let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        p.unflushed += 1;
        self.cv.notify_one();
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }

    /// `runFlushLoop`: wait for a line (or 30 s), give more lines 1 s to arrive, flush.
    fn run_flusher(self: Arc<Self>) {
        loop {
            {
                let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
                if p.unflushed == 0 && !p.exit {
                    p = self
                        .cv
                        .wait_timeout(p, Duration::from_secs(30))
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
                if p.exit {
                    drop(p);
                    self.flush();
                    return;
                }
                if p.unflushed == 0 {
                    continue;
                }
            }
            thread::sleep(Duration::from_millis(LOG_FLUSH_DEBOUNCE_MS));
            let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            self.flush();
            p.unflushed = 0;
        }
    }
}

enum Output {
    File(Arc<FileSink>),
    Stdout,
}

struct EngineLayer {
    output: Output,
    pid: u32,
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
    extra: String,
    critical: bool,
}

impl Visit for MessageVisitor {
    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "critical" {
            self.critical = value;
        } else {
            self.record_debug(field, &value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.extra.push_str(&format!(" {}={}", field.name(), value));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message.push_str(&format!("{value:?}"));
        } else {
            self.extra
                .push_str(&format!(" {}={:?}", field.name(), value));
        }
    }
}

fn level_name(level: &Level, critical: bool) -> &'static str {
    if critical {
        return "critical";
    }
    match *level {
        Level::ERROR => "error",
        Level::WARN => "warning",
        Level::INFO => "info",
        Level::DEBUG => "debug",
        Level::TRACE => "trace",
    }
}

fn is_engine_target(target: &str) -> bool {
    target.starts_with("ms_") || target.starts_with("mailsync")
}

/// `<pid> [YYYY-mm-dd HH:MM:SS.mmm] [<thread>] [<level>] <message>` plus the line ending.
pub fn format_file_line(
    pid: u32,
    time: &chrono::DateTime<chrono::Local>,
    thread: &str,
    level: &str,
    message: &str,
) -> String {
    format!(
        "{pid} [{}] [{thread}] [{level}] {message}{EOL}",
        time.format("%Y-%m-%d %H:%M:%S%.3f")
    )
}

impl<S: Subscriber> Layer<S> for EngineLayer {
    fn enabled(&self, metadata: &tracing::Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        *metadata.level() <= Level::INFO && is_engine_target(metadata.target())
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        if !(*meta.level() <= Level::INFO && is_engine_target(meta.target())) {
            return;
        }
        let mut v = MessageVisitor::default();
        event.record(&mut v);
        let mut message = v.message;
        message.push_str(&v.extra);
        let level = level_name(meta.level(), v.critical);

        match &self.output {
            Output::File(sink) => {
                let current = thread::current();
                let line = format_file_line(
                    self.pid,
                    &chrono::Local::now(),
                    current.name().unwrap_or(""),
                    level,
                    &message,
                );
                sink.write(&line);
                if v.critical {
                    sink.flush();
                    let _ = io::stderr().write_all(line.as_bytes());
                }
            }
            Output::Stdout => {
                let line = format!("{level}: {message}{EOL}");
                let _ = io::stdout().lock().write_all(line.as_bytes());
                if v.critical {
                    let _ = io::stderr().write_all(line.as_bytes());
                }
            }
        }
    }
}

static FILE_SINK: OnceLock<Arc<FileSink>> = OnceLock::new();

/// Installs the rotating file logger as the global subscriber. Call once, from `main`.
pub fn init_file(path: &Path) -> io::Result<()> {
    let file = RotatingFile::open(path, LOG_MAX_BYTES, LOG_MAX_FILES)?;
    let sink = Arc::new(FileSink {
        file: Mutex::new(file),
        pending: Mutex::new(FlushState::default()),
        cv: Condvar::new(),
    });
    let flusher = sink.clone();
    thread::Builder::new()
        .name("logFlush".into())
        .spawn(move || flusher.run_flusher())?;
    let _ = FILE_SINK.set(sink.clone());
    install(Output::File(sink))
}

/// Installs the `<level>: <message>` stdout logger as the global subscriber.
pub fn init_stdout() -> io::Result<()> {
    install(Output::Stdout)
}

fn install(output: Output) -> io::Result<()> {
    let layer = EngineLayer {
        output,
        pid: std::process::id(),
    };
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::set_global_default(subscriber).map_err(|e| io::Error::other(e.to_string()))
}

/// Writes buffered lines now. Call before `abort` / `exit`, which skip destructors.
pub fn flush() {
    if let Some(sink) = FILE_SINK.get() {
        sink.flush();
    }
    let _ = io::stdout().flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_format() {
        use chrono::TimeZone;
        let t = chrono::Local
            .with_ymd_and_hms(2026, 10, 14, 9, 5, 7)
            .unwrap()
            + chrono::Duration::milliseconds(42);
        let line = format_file_line(4242, &t, "background", "info", "Sync loop complete.");
        assert_eq!(
            line,
            format!("4242 [2026-10-14 09:05:07.042] [background] [info] Sync loop complete.{EOL}")
        );
    }

    #[test]
    fn rotation_names_and_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("mailsync-x.log");
        assert_eq!(rotated_path(&base, 2), dir.path().join("mailsync-x.log.2"));
        let mut f = RotatingFile::open(&base, 100, 3).unwrap();
        let line = format!("{}\n", "a".repeat(39)); // 40 bytes
        for _ in 0..12 {
            f.write_line(&line).unwrap();
        }
        f.flush().unwrap();
        // 12 lines of 40 bytes, 2 per file: base holds 2, .1 .. .3 hold 2 each, the rest
        // were dropped off the end.
        assert_eq!(fs::metadata(&base).unwrap().len(), 80);
        for i in 1..=3 {
            assert_eq!(fs::metadata(rotated_path(&base, i)).unwrap().len(), 80);
        }
        assert!(!rotated_path(&base, 4).exists());
    }

    #[test]
    fn reopening_counts_existing_size() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("m.log");
        fs::write(&base, "x".repeat(90)).unwrap();
        let mut f = RotatingFile::open(&base, 100, 3).unwrap();
        f.write_line(&"y".repeat(20)).unwrap();
        f.flush().unwrap();
        assert_eq!(fs::read_to_string(&base).unwrap(), "y".repeat(20));
        assert_eq!(
            fs::read_to_string(rotated_path(&base, 1)).unwrap(),
            "x".repeat(90)
        );
    }
}
