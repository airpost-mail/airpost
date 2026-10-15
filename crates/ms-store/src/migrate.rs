//! `migrate` and `reset` modes (docs/engine-port-plan.md section 9; `MailStore::migrate`,
//! `MailStore::resetForAccount`).
//!
//! The Rust engine does not upgrade C++ databases. A database the C++ engine wrote is moved
//! aside to `edgehill.legacy.db` (for `import-legacy`) and a fresh V12 database is created;
//! mail, contacts and events resync from the server.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use ms_core::constants::{
    DATABASE_FILE, FILES_DIR, LEGACY_DATABASE_FILE, SCHEMA_VERSION, STATE_ENGINE_KEY,
    STATE_ENGINE_RUST, VACUUM_INTERVAL, VACUUM_TIME_KEY,
};
use ms_core::{Clock, Error, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::schema;

/// What [`migrate`] did, for tests and the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MigrateOutcome {
    /// A database from another engine was moved to `edgehill.legacy.db`.
    pub moved_legacy: bool,
    /// A new V12 database was created.
    pub created: bool,
    pub vacuumed: bool,
}

/// Runs `--mode migrate`. Progress lines go to `out`, which is stdout in the bin: the
/// client opens its progress window when it sees `running migration` or `running vacuum`
/// (`mailsync-process.ts:501-517`). The final `{"error":...}` line is the caller's.
pub fn migrate(
    config_dir: &Path,
    clock: &dyn Clock,
    out: &mut dyn Write,
) -> Result<MigrateOutcome> {
    let mut outcome = MigrateOutcome::default();
    let db_path = config_dir.join(DATABASE_FILE);

    if db_path.exists() && !is_rust_database(&db_path) {
        // Note: printed before the move so the client's window is up while it happens.
        write_progress(out, "\nRunning Migration")?;
        move_legacy_database(config_dir)?;
        outcome.moved_legacy = true;
    }

    let conn = schema::open_connection(config_dir)?;
    schema::set_temp_directory(&conn, config_dir)?;
    let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;

    if version == 0 {
        if !outcome.moved_legacy {
            // The C++ engine printed this for a new database too; the client ignores it.
            write_progress(out, "\nRunning Setup")?;
        }
        schema::create_v12(&conn)?;
        save_key_value(&conn, STATE_ENGINE_KEY, STATE_ENGINE_RUST)?;
        save_key_value(&conn, VACUUM_TIME_KEY, &clock.now().to_string())?;
        outcome.created = true;
        // Bodies and attachments of the old database are re-downloaded under new ids.
        clear_dir_contents(&config_dir.join(FILES_DIR))?;
    } else if version < SCHEMA_VERSION {
        return Err(Error::other(format!(
            "edgehill.db is marked as written by this engine but has schema version {version}"
        )));
    }
    // Note: a version above SCHEMA_VERSION comes from a newer build; leave it alone, as the
    // C++ engine did.

    let vacuum_time = get_key_value(&conn, VACUUM_TIME_KEY)?
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(0);
    if clock.now() - vacuum_time > VACUUM_INTERVAL {
        write_progress(out, "\nRunning Vacuum\n")?;
        // Updated first so a VACUUM that fails (usually a full disk) is not retried on
        // every launch.
        save_key_value(&conn, VACUUM_TIME_KEY, &clock.now().to_string())?;
        if let Err(e) = conn.execute_batch("VACUUM") {
            // Not mandatory: report it and let the app launch.
            write_progress(out, &format!("\nVacuuming failed with SQLite error:\n{e}"))?;
        }
        outcome.vacuumed = true;
    }
    Ok(outcome)
}

/// `--mode reset`: deletes everything belonging to one account and resets its metadata
/// cursor so metadata is fetched again on resync.
pub fn reset_account(conn: &Connection, account_id: &str) -> Result<()> {
    for sql in schema::ACCOUNT_RESET_QUERIES {
        conn.execute(sql, [account_id])?;
    }
    save_key_value(conn, &format!("cursor-{account_id}"), "0")?;
    conn.execute_batch("VACUUM")?;
    Ok(())
}

/// True when `_State.engine` says this engine wrote the database. Anything unreadable
/// counts as legacy: it is moved aside rather than deleted.
fn is_rust_database(db_path: &Path) -> bool {
    let Ok(conn) = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE) else {
        return false;
    };
    let engine: Option<String> = conn
        .query_row(
            "SELECT value FROM `_State` WHERE id = ?",
            [STATE_ENGINE_KEY],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    engine.as_deref() == Some(STATE_ENGINE_RUST)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Moves `edgehill.db`, `-wal` and `-shm` to `edgehill.legacy.db*` together, so a WAL that
/// was not checkpointed stays with its database. An older legacy copy is replaced.
fn move_legacy_database(config_dir: &Path) -> Result<()> {
    let from = config_dir.join(DATABASE_FILE);
    let to = config_dir.join(LEGACY_DATABASE_FILE);
    for suffix in ["", "-wal", "-shm"] {
        let target = with_suffix(&to, suffix);
        if target.exists() {
            fs::remove_file(&target)?;
        }
    }
    for suffix in ["", "-wal", "-shm"] {
        let source = with_suffix(&from, suffix);
        if source.exists() {
            fs::rename(&source, with_suffix(&to, suffix))?;
        }
    }
    Ok(())
}

fn clear_dir_contents(dir: &Path) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn write_progress(out: &mut dyn Write, text: &str) -> Result<()> {
    out.write_all(text.as_bytes())?;
    out.flush()?;
    Ok(())
}

pub fn get_key_value(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM `_State` WHERE id = ?", [key], |r| {
            r.get(0)
        })
        .optional()?)
}

pub fn save_key_value(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "REPLACE INTO `_State` (id, value) VALUES (?, ?)",
        [key, value],
    )?;
    Ok(())
}
