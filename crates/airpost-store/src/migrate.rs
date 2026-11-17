// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `migrate` and `reset` modes. Derived from
//! Mailspring-Sync `MailStore::migrate` and `MailStore::resetForAccount`, Copyright © 2017
//! Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! The Rust engine does not upgrade C++ databases. A database the C++ engine wrote (Mailspring's
//! `edgehill.db`, which the desktop's Import from Mailspring copies to `airpost.db`) is moved
//! aside to `airpost.legacy.db` (for `import-legacy`) and a fresh database is created;
//! mail, contacts and events resync from the server.
//!
//! Airpost profiles made before the database was called `airpost.db` hold it as `edgehill.db`
//! (and a moved-aside one as `edgehill.legacy.db`); `migrate` renames those first.
//!
//! An Airpost database is upgraded in place: `user_version` is 12 for the V12 schema and goes up
//! by one with each of [`schema::MIGRATIONS`]. A database from a newer build is refused.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use airpost_core::constants::{
    BASE_SCHEMA_VERSION, DATABASE_FILE, FILES_DIR, LEGACY_DATABASE_FILE, SCHEMA_VERSION,
    STATE_ENGINE_KEY, STATE_ENGINE_RUST, VACUUM_TIME_KEY,
};
use airpost_core::{Clock, Error, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior};

use crate::schema;

/// The names an Airpost profile's databases had before `airpost.db` and `airpost.legacy.db`.
const OLD_DATABASE_NAMES: [(&str, &str); 2] = [
    ("edgehill.db", DATABASE_FILE),
    ("edgehill.legacy.db", LEGACY_DATABASE_FILE),
];

/// What [`migrate`] did, for tests and the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MigrateOutcome {
    /// A database from another engine was moved to `airpost.legacy.db`.
    pub moved_legacy: bool,
    /// A new database was created.
    pub created: bool,
    /// An existing Airpost database was brought up to [`SCHEMA_VERSION`].
    pub upgraded: bool,
}

/// Runs `--mode migrate` (and `serve` before it accepts connections). Progress lines go to
/// `out`, which is stdout in the bin: the client opens its progress window when it sees
/// `running migration` (`apps/desktop/src/engine/process.ts`). The final `{"error":...}` line is
/// the caller's.
///
/// There is no full VACUUM here any more: a new database uses `auto_vacuum =
/// INCREMENTAL` and the background worker frees pages a little at a time
/// ([`crate::Store::compact`]). A database created before that keeps its free pages until
/// a VACUUM converts it.
pub fn migrate(
    config_dir: &Path,
    clock: &dyn Clock,
    out: &mut dyn Write,
) -> Result<MigrateOutcome> {
    let mut outcome = MigrateOutcome::default();
    rename_old_databases(config_dir)?;
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
        // The WAL pragma already wrote the header, so the mode takes a VACUUM, instant on
        // an empty database.
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM;")?;
        if !outcome.moved_legacy {
            // The C++ engine printed this for a new database too; the client ignores it.
            write_progress(out, "\nRunning Setup")?;
        }
        schema::create_v12(&conn)?;
        save_key_value(&conn, STATE_ENGINE_KEY, STATE_ENGINE_RUST)?;
        save_key_value(&conn, VACUUM_TIME_KEY, &clock.now().to_string())?;
        outcome.created = true;
        if outcome.moved_legacy {
            // Local drafts' attachments exist nowhere else; import-legacy copies them back.
            if let Err(e) = crate::legacy_import::set_aside_draft_files(config_dir) {
                write_progress(out, &format!("\nCould not keep draft attachments: {e}"))?;
            }
        }
        // Bodies and attachments of the old database are re-downloaded under new ids.
        clear_dir_contents(&config_dir.join(FILES_DIR))?;
    } else if version < BASE_SCHEMA_VERSION {
        return Err(Error::other(format!(
            "airpost.db is marked as written by this engine but has schema version {version}"
        )));
    } else if version > SCHEMA_VERSION {
        // An older build would write rows without what the newer schema expects of them.
        return Err(Error::other(format!(
            "airpost.db has schema version {version}, newer than this build's {SCHEMA_VERSION}; update Airpost to open it"
        )));
    } else if version < SCHEMA_VERSION {
        write_progress(out, "\nRunning Migration")?;
        outcome.upgraded = true;
    }
    apply_migrations(&conn)?;

    Ok(outcome)
}

/// Applies the [`schema::MIGRATIONS`] above the database's version, each in an IMMEDIATE
/// transaction that re-reads `user_version`, so a second process migrating the same database
/// waits for the first and skips what it applied.
fn apply_migrations(conn: &Connection) -> Result<()> {
    for migration in schema::MIGRATIONS {
        let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
        let version: i32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version >= migration.version {
            continue;
        }
        if version != migration.version - 1 {
            return Err(Error::other(format!(
                "airpost.db has schema version {version}, expected {}",
                migration.version - 1
            )));
        }
        tx.execute_batch(migration.sql)?;
        tx.pragma_update(None, "user_version", migration.version)?;
        tx.commit()?;
        tracing::info!(
            "- Migrated airpost.db to schema version {}",
            migration.version
        );
    }
    Ok(())
}

/// `--mode reset`: deletes everything belonging to one account and resets its metadata
/// cursor so metadata is fetched again on resync. With `keep_metadata` (a reset that resyncs
/// the account) its plugin metadata is kept in `DetatchedPluginMetadata` by Message-ID and
/// attached again as the resync brings the messages back ([`crate::metadata_stash`]), and
/// `ACCOUNT_RESET_QUERIES`' delete of that table is skipped. Without it (the account is being
/// removed) the metadata goes too: account ids are derived from the address and hosts, so
/// metadata kept for a removed account would come back to life if it were added again.
pub fn reset_account(conn: &Connection, account_id: &str, keep_metadata: bool) -> Result<()> {
    let stash = if keep_metadata {
        crate::metadata_stash::collect(conn, Some(account_id))?
    } else {
        Vec::new()
    };
    let tx = conn.unchecked_transaction()?;
    for sql in schema::ACCOUNT_RESET_QUERIES
        .iter()
        .filter(|sql| !keep_metadata || !sql.contains("`DetatchedPluginMetadata`"))
    {
        tx.execute(sql, [account_id])?;
    }
    crate::metadata_stash::save(&tx, &stash)?;
    save_key_value(&tx, &format!("cursor-{account_id}"), "0")?;
    tx.commit()?;
    crate::schema::compact(conn);
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

/// Moves `airpost.db`, `-wal` and `-shm` to `airpost.legacy.db*` together, so a WAL that
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

/// Renames a database under its old name (with its `-wal` and `-shm`) to the current one,
/// unless a database already has the current name. The main file goes last, so a rename cut
/// short is finished by the next run.
fn rename_old_databases(config_dir: &Path) -> Result<()> {
    for (old, new) in OLD_DATABASE_NAMES {
        let from = config_dir.join(old);
        let to = config_dir.join(new);
        if !from.exists() || to.exists() {
            continue;
        }
        for suffix in ["-wal", "-shm", ""] {
            let source = with_suffix(&from, suffix);
            if source.exists() {
                fs::rename(&source, with_suffix(&to, suffix))?;
            }
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
