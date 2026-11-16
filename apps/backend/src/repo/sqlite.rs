// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! [`Repo`] over rusqlite: one writer connection, so commit order is seq order (3.5), and a
//! small pool of read-only connections. WAL lets readers run beside the writer.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Row, Transaction, TransactionBehavior,
};

use super::{metadata, schema};
use super::{
    normalize_email, Authenticated, ClaimOutcome, MetadataWrite, NewUser, Repo, RepoError, Result,
    TokenInfo, User, Written,
};
use crate::ids;
use crate::wire::{MetadataRecord, MAX_ACCOUNTS_PER_USER};

const MAX_IDLE_READERS: usize = 4;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct SqliteRepo {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic inside a closure leaves the connection usable; rusqlite rolls back the
    // transaction on drop.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl SqliteRepo {
    /// Opens (creating if needed) `airpost.db` in `data_dir` and applies pending migrations.
    pub fn open(data_dir: &Path) -> Result<SqliteRepo> {
        Self::open_with(data_dir, schema::MIGRATIONS)
    }

    /// Before upgrading an existing database, writes a copy of it to
    /// `pre-migrate-<from>-to-<to>-<time>.db` in `data_dir`, so an upgrade can be rolled back
    /// by restoring the copy and running the previous version. Like [`Repo::backup`], the copy
    /// has a new server id.
    fn open_with(data_dir: &Path, migrations: &[&str]) -> Result<SqliteRepo> {
        create_private_dir(data_dir)?;
        let path = data_dir.join("airpost.db");
        make_db_private(&path)?;
        let mut writer = Connection::open(&path)?;
        writer.busy_timeout(BUSY_TIMEOUT)?;
        writer.pragma_update(None, "journal_mode", "WAL")?;
        writer.pragma_update(None, "synchronous", "NORMAL")?;
        writer.pragma_update(None, "foreign_keys", true)?;
        let from = schema::schema_version(&writer)?;
        let to = migrations.len() as u32;
        if from > 0 && from < to {
            let copy = data_dir.join(format!(
                "pre-migrate-{from}-to-{to}-{}.db",
                ids::file_time(ids::now_ms())
            ));
            vacuum_into(&path, &copy)?;
            tracing::info!(copy = %copy.display(), from, to, "copied the database before migrating");
        }
        schema::migrate(&mut writer, migrations, ids::new_server_id)?;
        Ok(SqliteRepo {
            path,
            writer: Mutex::new(writer),
            readers: Mutex::new(Vec::new()),
        })
    }

    /// Locks `<data_dir>/serve.lock` for as long as the returned file is open. `serve` holds
    /// it, so a second server can't share the data dir and `restore` and `rotate-server-id`
    /// refuse to run under a live one.
    pub fn lock_data_dir(data_dir: &Path) -> Result<std::fs::File> {
        create_private_dir(data_dir)?;
        let mut o = std::fs::OpenOptions::new();
        o.create(true).write(true).truncate(false);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let file = o.open(data_dir.join("serve.lock"))?;
        match file.try_lock() {
            Ok(()) => Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => Err(RepoError::Locked(data_dir.to_owned())),
            Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
        }
    }

    /// Replaces `airpost.db` in `data_dir` with a copy of the database at `src` (a backup, a
    /// pre-migration copy, or a file-level copy of a stopped server's `airpost.db`), removing
    /// any `-wal` and `-shm` left beside the old one. The restored database gets a new server
    /// id, which is returned, so clients re-sync instead of skipping writes. The caller holds
    /// [`SqliteRepo::lock_data_dir`].
    pub fn restore(data_dir: &Path, src: &Path) -> Result<String> {
        let live = data_dir.join("airpost.db");
        if live.exists() && std::fs::canonicalize(&live)? == std::fs::canonicalize(src)? {
            return Err(RepoError::NotRestorable(format!(
                "{} is the live database",
                src.display()
            )));
        }
        {
            let conn = Connection::open_with_flags(src, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let not_ours = || {
                RepoError::NotRestorable(format!(
                    "{} is not a airpost-backend database",
                    src.display()
                ))
            };
            let version = schema::schema_version(&conn).map_err(|_| not_ours())?;
            if version == 0 {
                return Err(not_ours());
            }
            if version > schema::MIGRATIONS.len() as u32 {
                return Err(schema::MigrateError::TooNew {
                    found: version,
                    supported: schema::MIGRATIONS.len() as u32,
                }
                .into());
            }
            conn.query_row("SELECT value FROM meta WHERE key = 'server_id'", [], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|_| not_ours())?;
        }
        create_private_dir(data_dir)?;
        let tmp = data_dir.join("airpost.db.restoring");
        let _ = std::fs::remove_file(&tmp);
        create_private_file(&tmp)?;
        let copy = || -> Result<String> {
            // Through SQLite rather than a file copy, so a source still in WAL mode with its
            // own -wal file is copied whole.
            let conn = Connection::open_with_flags(src, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            conn.execute(
                "VACUUM INTO ?1",
                [tmp.to_str().ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "data dir is not UTF-8")
                })?],
            )?;
            drop(conn);
            let id = ids::new_server_id();
            set_server_id(&Connection::open(&tmp)?, &id)?;
            Ok(id)
        };
        let id = copy().inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        for ext in ["-wal", "-shm"] {
            match std::fs::remove_file(data_dir.join(format!("airpost.db{ext}"))) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        std::fs::rename(&tmp, &live)?;
        Ok(id)
    }

    fn open_reader(&self) -> Result<Connection> {
        let conn = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        Ok(conn)
    }

    fn read<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let idle = lock(&self.readers).pop();
        let conn = match idle {
            Some(c) => c,
            None => self.open_reader()?,
        };
        let out = f(&conn);
        let mut pool = lock(&self.readers);
        if pool.len() < MAX_IDLE_READERS {
            pool.push(conn);
        }
        Ok(out?)
    }

    fn write<T>(&self, f: impl FnOnce(&Transaction) -> Result<T>) -> Result<T> {
        let mut conn = lock(&self.writer);
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }
}

/// Creates `dir` (and missing parents) readable by the owner only. An existing directory is
/// left as it is; the files inside are restricted by [`make_db_private`].
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

/// Creates an empty file only the owner can read; fails if `path` exists.
fn create_private_file(path: &Path) -> std::io::Result<()> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path).map(drop)
}

/// The database holds password and token hashes. SQLite gives `-wal` and `-shm` the mode of
/// the main file, so creating it 0600 (and tightening an existing one) covers all three.
fn make_db_private(path: &Path) -> std::io::Result<()> {
    match create_private_file(path) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        other => return other,
    }
    #[cfg(unix)]
    for p in [
        path.to_owned(),
        path.with_extension("db-wal"),
        path.with_extension("db-shm"),
    ] {
        use std::os::unix::fs::PermissionsExt;
        let Ok(meta) = std::fs::metadata(&p) else {
            continue;
        };
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            match std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode & 0o700)) {
                Ok(()) => tracing::info!(path = %p.display(), "made database file owner-only"),
                Err(e) => {
                    tracing::warn!(path = %p.display(), error = %e, "database file is readable by others")
                }
            }
        }
    }
    Ok(())
}

fn set_server_id(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE meta SET value = ?1 WHERE key = 'server_id'", [id])?;
    Ok(())
}

const USER_COLUMNS: &str =
    "u.id, u.email, u.first_name, u.last_name, u.tracking_ns, u.created_at, u.disabled_at";

fn user_from_row(r: &Row, offset: usize) -> rusqlite::Result<User> {
    Ok(User {
        id: r.get(offset)?,
        email: r.get(offset + 1)?,
        first_name: r.get(offset + 2)?,
        last_name: r.get(offset + 3)?,
        tracking_ns: r.get(offset + 4)?,
        created_at: r.get(offset + 5)?,
        disabled_at: r.get(offset + 6)?,
    })
}

/// Creates `(user, aid)` on first use, within the caller's transaction.
fn ensure_account(tx: &Transaction, user_id: &str, aid: &str, now: i64) -> Result<()> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM accounts WHERE user_id = ?1 AND aid = ?2)",
        params![user_id, aid],
        |r| r.get(0),
    )?;
    if exists {
        return Ok(());
    }
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM accounts WHERE user_id = ?1",
        [user_id],
        |r| r.get(0),
    )?;
    if count >= MAX_ACCOUNTS_PER_USER as i64 {
        return Err(RepoError::AccountLimit);
    }
    tx.execute(
        "INSERT INTO accounts (user_id, aid, created_at) VALUES (?1, ?2, ?3)",
        params![user_id, aid, now],
    )?;
    Ok(())
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _)
            if f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    )
}

impl Repo for SqliteRepo {
    fn schema_version(&self) -> Result<u32> {
        self.read(schema::schema_version)
    }

    fn server_id(&self) -> Result<String> {
        self.read(|c| {
            c.query_row("SELECT value FROM meta WHERE key = 'server_id'", [], |r| {
                r.get(0)
            })
        })
    }

    fn rotate_server_id(&self) -> Result<String> {
        let id = ids::new_server_id();
        self.write(|tx| Ok(set_server_id(tx, &id)?))?;
        Ok(id)
    }

    fn current_seq(&self) -> Result<i64> {
        self.read(|c| c.query_row("SELECT value FROM seq WHERE id = 1", [], |r| r.get(0)))
    }

    fn create_user(&self, new: &NewUser, now: i64) -> Result<User> {
        let user = User {
            id: ids::new_user_id(),
            email: normalize_email(&new.email),
            first_name: new.first_name.trim().to_owned(),
            last_name: new.last_name.trim().to_owned(),
            tracking_ns: ids::new_tracking_ns(),
            created_at: now,
            disabled_at: None,
        };
        self.write(|tx| {
            tx.execute(
                "INSERT INTO users (id, email, first_name, last_name, password_hash, tracking_ns, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    user.id,
                    user.email,
                    user.first_name,
                    user.last_name,
                    new.password_hash,
                    user.tracking_ns,
                    user.created_at
                ],
            )
            .map_err(|e| {
                // The random id and namespace collide with negligible probability; a unique
                // violation here is the email.
                if is_unique_violation(&e) {
                    RepoError::EmailTaken
                } else {
                    e.into()
                }
            })?;
            Ok(())
        })?;
        Ok(user)
    }

    fn user_by_id(&self, id: &str) -> Result<Option<User>> {
        self.read(|c| {
            c.query_row(
                &format!("SELECT {USER_COLUMNS} FROM users u WHERE u.id = ?1"),
                [id],
                |r| user_from_row(r, 0),
            )
            .optional()
        })
    }

    fn user_by_email(&self, email: &str) -> Result<Option<User>> {
        Ok(self.login_candidate(email)?.map(|(u, _)| u))
    }

    fn login_candidate(&self, email: &str) -> Result<Option<(User, String)>> {
        let email = normalize_email(email);
        self.read(|c| {
            c.query_row(
                &format!("SELECT {USER_COLUMNS}, u.password_hash FROM users u WHERE u.email = ?1"),
                [email],
                |r| Ok((user_from_row(r, 0)?, r.get(7)?)),
            )
            .optional()
        })
    }

    fn list_users(&self) -> Result<Vec<User>> {
        self.read(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {USER_COLUMNS} FROM users u ORDER BY u.created_at, u.email"
            ))?;
            let rows = stmt.query_map([], |r| user_from_row(r, 0))?;
            rows.collect()
        })
    }

    fn set_user_disabled(&self, user_id: &str, disabled: bool, now: i64) -> Result<bool> {
        self.write(|tx| {
            let n = if disabled {
                tx.execute(
                    "UPDATE users SET disabled_at = COALESCE(disabled_at, ?2) WHERE id = ?1",
                    params![user_id, now],
                )?
            } else {
                tx.execute(
                    "UPDATE users SET disabled_at = NULL WHERE id = ?1",
                    [user_id],
                )?
            };
            Ok(n > 0)
        })
    }

    fn delete_user(&self, user_id: &str) -> Result<bool> {
        // Tokens, accounts (and through them metadata and claims) and tracking events cascade.
        self.write(|tx| Ok(tx.execute("DELETE FROM users WHERE id = ?1", [user_id])? > 0))
    }

    fn create_token(&self, user_id: &str, hash: &[u8; 32], name: &str, now: i64) -> Result<i64> {
        self.write(|tx| {
            tx.execute(
                "INSERT INTO tokens (user_id, hash, name, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![user_id, &hash[..], name, now],
            )?;
            Ok(tx.last_insert_rowid())
        })
    }

    fn authenticate(&self, hash: &[u8; 32]) -> Result<Option<Authenticated>> {
        self.read(|c| {
            c.query_row(
                &format!(
                    "SELECT t.id, t.last_used_at, {USER_COLUMNS}
                     FROM tokens t JOIN users u ON u.id = t.user_id
                     WHERE t.hash = ?1 AND t.revoked_at IS NULL AND u.disabled_at IS NULL"
                ),
                [&hash[..]],
                |r| {
                    Ok(Authenticated {
                        token_id: r.get(0)?,
                        token_last_used_at: r.get(1)?,
                        user: user_from_row(r, 2)?,
                    })
                },
            )
            .optional()
        })
    }

    fn touch_token(&self, token_id: i64, now: i64) -> Result<()> {
        self.write(|tx| {
            tx.execute(
                "UPDATE tokens SET last_used_at = ?2 WHERE id = ?1",
                params![token_id, now],
            )?;
            Ok(())
        })
    }

    fn list_tokens(&self, user_id: Option<&str>) -> Result<Vec<TokenInfo>> {
        self.read(|c| {
            let mut stmt = c.prepare(
                "SELECT t.id, t.user_id, u.email, t.name, t.created_at, t.last_used_at, t.revoked_at
                 FROM tokens t JOIN users u ON u.id = t.user_id
                 WHERE ?1 IS NULL OR t.user_id = ?1
                 ORDER BY t.id",
            )?;
            let rows = stmt.query_map([user_id], |r| {
                Ok(TokenInfo {
                    id: r.get(0)?,
                    user_id: r.get(1)?,
                    user_email: r.get(2)?,
                    name: r.get(3)?,
                    created_at: r.get(4)?,
                    last_used_at: r.get(5)?,
                    revoked_at: r.get(6)?,
                })
            })?;
            rows.collect()
        })
    }

    fn revoke_token(&self, token_id: i64, now: i64) -> Result<bool> {
        self.write(|tx| {
            Ok(tx.execute(
                "UPDATE tokens SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
                params![token_id, now],
            )? > 0)
        })
    }

    fn ensure_account(&self, user_id: &str, aid: &str, now: i64) -> Result<()> {
        self.write(|tx| ensure_account(tx, user_id, aid, now))
    }

    fn delete_account(&self, user_id: &str, aid: &str) -> Result<bool> {
        self.write(|tx| {
            // tracking_events has no aid; attribute events through the account's tracked rows.
            // Pending events whose row was never pushed belong to no account and age out with
            // retention.
            tx.execute(
                "DELETE FROM tracking_events
                 WHERE user_id = ?1 AND tracking_id IN (
                     SELECT tracking_id FROM metadata
                     WHERE user_id = ?1 AND aid = ?2 AND tracking_id IS NOT NULL)",
                params![user_id, aid],
            )?;
            // metadata and claims cascade.
            Ok(tx.execute(
                "DELETE FROM accounts WHERE user_id = ?1 AND aid = ?2",
                params![user_id, aid],
            )? > 0)
        })
    }

    fn push_metadata(
        &self,
        user_id: &str,
        aid: &str,
        w: &MetadataWrite,
        now: i64,
    ) -> Result<Written> {
        self.write(|tx| {
            ensure_account(tx, user_id, aid, now)?;
            Ok(metadata::push(tx, user_id, aid, w, now)?)
        })
    }

    fn backfill_metadata(
        &self,
        user_id: &str,
        aid: &str,
        records: &[MetadataWrite],
        now: i64,
    ) -> Result<Vec<Written>> {
        self.write(|tx| {
            ensure_account(tx, user_id, aid, now)?;
            let mut out = Vec::new();
            for w in records {
                out.extend(metadata::backfill(tx, user_id, aid, w, now)?);
            }
            Ok(out)
        })
    }

    fn metadata_head(&self, user_id: &str, aid: &str) -> Result<i64> {
        self.read(|c| metadata::head(c, user_id, aid))
    }

    fn metadata_page(
        &self,
        user_id: &str,
        aid: &str,
        limit: usize,
        offset: u64,
    ) -> Result<Vec<MetadataRecord>> {
        self.read(|c| metadata::page(c, user_id, aid, limit, offset))
    }

    fn metadata_since(
        &self,
        user_id: &str,
        aid: &str,
        after: i64,
        limit: usize,
    ) -> Result<Vec<(i64, MetadataRecord)>> {
        self.read(|c| metadata::since(c, user_id, aid, after, limit))
    }

    fn claim_expiration(
        &self,
        user_id: &str,
        aid: &str,
        object_id: &str,
        plugin_id: &str,
        expiration: i64,
        token_id: i64,
        now: i64,
    ) -> Result<ClaimOutcome> {
        self.write(|tx| {
            ensure_account(tx, user_id, aid, now)?;
            let inserted = tx.execute(
                "INSERT INTO claims (user_id, aid, object_id, plugin_id, expiration, token_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT DO NOTHING",
                params![user_id, aid, object_id, plugin_id, expiration, token_id, now],
            )?;
            if inserted > 0 {
                return Ok(ClaimOutcome::Granted);
            }
            // The holder asking again (a retry after a lost response) still holds it.
            let holder: Option<i64> = tx.query_row(
                "SELECT token_id FROM claims
                 WHERE user_id = ?1 AND aid = ?2 AND object_id = ?3 AND plugin_id = ?4 AND expiration = ?5",
                params![user_id, aid, object_id, plugin_id, expiration],
                |r| r.get(0),
            )?;
            Ok(if holder == Some(token_id) {
                ClaimOutcome::Granted
            } else {
                ClaimOutcome::Taken
            })
        })
    }

    fn purge_claims(&self, before: i64) -> Result<usize> {
        self.write(|tx| Ok(tx.execute("DELETE FROM claims WHERE created_at < ?1", [before])?))
    }

    fn live_token_user(&self, token_id: i64) -> Result<Option<User>> {
        self.read(|c| {
            c.query_row(
                &format!(
                    "SELECT {USER_COLUMNS}
                     FROM tokens t JOIN users u ON u.id = t.user_id
                     WHERE t.id = ?1 AND t.revoked_at IS NULL AND u.disabled_at IS NULL"
                ),
                [token_id],
                |r| user_from_row(r, 0),
            )
            .optional()
        })
    }

    fn backup(&self, dest: &Path) -> Result<String> {
        vacuum_into(&self.path, dest)
    }
}

/// Writes a consistent, owner-only copy of the database at `src` to `dest`, which must not
/// exist, and gives the copy a new server id, which it returns.
fn vacuum_into(src: &Path, dest: &Path) -> Result<String> {
    let dest_str = dest.to_str().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "backup path is not UTF-8")
    })?;
    // VACUUM INTO accepts an empty file, so the copy gets this file's owner-only mode.
    create_private_file(dest).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            std::io::Error::new(e.kind(), format!("{} already exists", dest.display()))
        } else {
            e
        }
    })?;
    let copy = || -> Result<String> {
        // A separate connection, so the backup does not hold the writer.
        let conn = Connection::open(src)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.execute("VACUUM INTO ?1", [dest_str])?;
        drop(conn);
        // The copy's seq stops here while the live server goes on. Restored under the
        // same id, engines would keep cursors past the restored counter and skip writes.
        let id = ids::new_server_id();
        set_server_id(&Connection::open(dest)?, &id)?;
        Ok(id)
    };
    copy().inspect_err(|_| {
        let _ = std::fs::remove_file(dest);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::SCHEMA_VERSION;

    fn repo() -> (tempfile::TempDir, SqliteRepo) {
        let dir = tempfile::tempdir().unwrap();
        let repo = SqliteRepo::open(dir.path()).unwrap();
        (dir, repo)
    }

    fn new_user(email: &str) -> NewUser {
        NewUser {
            email: email.into(),
            first_name: " Ada ".into(),
            last_name: "Lovelace".into(),
            password_hash: "$argon2id$fake".into(),
        }
    }

    #[test]
    fn migrate_is_idempotent_and_keeps_server_id() {
        let dir = tempfile::tempdir().unwrap();
        let id = {
            let repo = SqliteRepo::open(dir.path()).unwrap();
            assert_eq!(repo.schema_version().unwrap(), SCHEMA_VERSION);
            repo.server_id().unwrap()
        };
        assert_eq!(id.len(), 32);
        let repo = SqliteRepo::open(dir.path()).unwrap();
        assert_eq!(repo.server_id().unwrap(), id);
        let other = tempfile::tempdir().unwrap();
        assert_ne!(
            SqliteRepo::open(other.path()).unwrap().server_id().unwrap(),
            id
        );
    }

    #[test]
    fn refuses_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        drop(SqliteRepo::open(dir.path()).unwrap());
        let c = Connection::open(dir.path().join("airpost.db")).unwrap();
        c.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(c);
        assert!(matches!(
            SqliteRepo::open(dir.path()),
            Err(RepoError::Migrate(schema::MigrateError::TooNew { .. }))
        ));
    }

    #[test]
    fn data_dir_lock() {
        let dir = tempfile::tempdir().unwrap();
        let held = SqliteRepo::lock_data_dir(dir.path()).unwrap();
        assert!(matches!(
            SqliteRepo::lock_data_dir(dir.path()),
            Err(RepoError::Locked(_))
        ));
        drop(held);
        SqliteRepo::lock_data_dir(dir.path()).unwrap();
    }

    #[test]
    fn restore_replaces_the_database() {
        let (dir, repo) = repo();
        repo.create_user(&new_user("ada@example.com"), 1).unwrap();
        let id = repo.server_id().unwrap();
        let out = tempfile::tempdir().unwrap();
        let copy = out.path().join("copy.db");
        repo.backup(&copy).unwrap();
        repo.create_user(&new_user("bob@example.com"), 2).unwrap();
        drop(repo);
        std::fs::write(dir.path().join("airpost.db-wal"), b"stale").unwrap();
        std::fs::write(dir.path().join("airpost.db-shm"), b"stale").unwrap();

        let restored_id = SqliteRepo::restore(dir.path(), &copy).unwrap();
        assert!(!dir.path().join("airpost.db-wal").exists());
        assert!(!dir.path().join("airpost.db.restoring").exists());
        let repo = SqliteRepo::open(dir.path()).unwrap();
        assert_eq!(repo.server_id().unwrap(), restored_id);
        assert_ne!(restored_id, id);
        let emails: Vec<_> = repo
            .list_users()
            .unwrap()
            .into_iter()
            .map(|u| u.email)
            .collect();
        assert_eq!(emails, ["ada@example.com"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("airpost.db"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // Not ours, newer than this build, or the live file itself.
        let junk = out.path().join("junk.db");
        std::fs::write(&junk, b"not a database").unwrap();
        assert!(SqliteRepo::restore(dir.path(), &junk).is_err());
        let empty = out.path().join("empty.db");
        drop(Connection::open(&empty).unwrap());
        assert!(matches!(
            SqliteRepo::restore(dir.path(), &empty),
            Err(RepoError::NotRestorable(_))
        ));
        let newer = out.path().join("newer.db");
        repo.backup(&newer).unwrap();
        Connection::open(&newer)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(matches!(
            SqliteRepo::restore(dir.path(), &newer),
            Err(RepoError::Migrate(schema::MigrateError::TooNew { .. }))
        ));
        assert!(matches!(
            SqliteRepo::restore(dir.path(), &dir.path().join("airpost.db")),
            Err(RepoError::NotRestorable(_))
        ));
        assert_eq!(repo.server_id().unwrap(), restored_id);
    }

    #[test]
    fn copies_before_migrating() {
        let pre_migrate = |dir: &Path| -> Vec<PathBuf> {
            std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.file_name().unwrap().to_str().unwrap().starts_with("pre-"))
                .collect()
        };
        let dir = tempfile::tempdir().unwrap();
        // A fresh database and an up-to-date one are not copied.
        let id = SqliteRepo::open(dir.path()).unwrap().server_id().unwrap();
        drop(SqliteRepo::open(dir.path()).unwrap());
        assert!(pre_migrate(dir.path()).is_empty());

        let mut next = schema::MIGRATIONS.to_vec();
        next.push("CREATE TABLE later (x INTEGER) STRICT;");
        let repo = SqliteRepo::open_with(dir.path(), &next).unwrap();
        assert_eq!(repo.schema_version().unwrap(), SCHEMA_VERSION + 1);
        assert_eq!(repo.server_id().unwrap(), id);

        let copies = pre_migrate(dir.path());
        assert_eq!(copies.len(), 1);
        let name = copies[0].file_name().unwrap().to_str().unwrap().to_owned();
        let prefix = format!("pre-migrate-{SCHEMA_VERSION}-to-{}-", SCHEMA_VERSION + 1);
        assert!(name.starts_with(&prefix), "{name}");
        let copy = Connection::open(&copies[0]).unwrap();
        assert_eq!(schema::schema_version(&copy).unwrap(), SCHEMA_VERSION);
        let copy_id: String = copy
            .query_row("SELECT value FROM meta WHERE key = 'server_id'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_ne!(copy_id, id);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&copies[0]).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn users() {
        let (_d, repo) = repo();
        let u = repo
            .create_user(&new_user(" Ada@Example.COM "), 1000)
            .unwrap();
        assert_eq!(u.email, "ada@example.com");
        assert_eq!(u.first_name, "Ada");
        assert!(matches!(
            repo.create_user(&new_user("ada@example.com"), 1),
            Err(RepoError::EmailTaken)
        ));
        assert_eq!(
            repo.user_by_email("ADA@example.com").unwrap(),
            Some(u.clone())
        );
        assert_eq!(repo.user_by_id(&u.id).unwrap(), Some(u.clone()));
        let (_, hash) = repo.login_candidate("ada@example.com").unwrap().unwrap();
        assert_eq!(hash, "$argon2id$fake");
        assert!(repo.user_by_email("nobody@example.com").unwrap().is_none());

        let b = repo.create_user(&new_user("b@example.com"), 2000).unwrap();
        assert_ne!(u.tracking_ns, b.tracking_ns);
        let all = repo.list_users().unwrap();
        assert_eq!(
            all.iter().map(|u| u.email.as_str()).collect::<Vec<_>>(),
            ["ada@example.com", "b@example.com"]
        );

        assert!(repo.set_user_disabled(&u.id, true, 5).unwrap());
        assert!(repo.set_user_disabled(&u.id, true, 9).unwrap());
        assert_eq!(
            repo.user_by_id(&u.id).unwrap().unwrap().disabled_at,
            Some(5)
        );
        assert!(repo.set_user_disabled(&u.id, false, 9).unwrap());
        assert_eq!(repo.user_by_id(&u.id).unwrap().unwrap().disabled_at, None);
        assert!(!repo.set_user_disabled("missing", true, 9).unwrap());
    }

    #[test]
    fn tokens_authenticate_until_revoked_or_disabled() {
        let (_d, repo) = repo();
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        let t1 = ids::hash_token("one");
        let t2 = ids::hash_token("two");
        let id1 = repo.create_token(&u.id, &t1, "laptop", 10).unwrap();
        let id2 = repo.create_token(&u.id, &t2, "desktop", 11).unwrap();

        let a = repo.authenticate(&t1).unwrap().unwrap();
        assert_eq!(a.user, u);
        assert_eq!(a.token_id, id1);
        assert_eq!(a.token_last_used_at, None);
        repo.touch_token(id1, 20).unwrap();
        assert_eq!(
            repo.authenticate(&t1).unwrap().unwrap().token_last_used_at,
            Some(20)
        );
        assert!(repo
            .authenticate(&ids::hash_token("three"))
            .unwrap()
            .is_none());

        assert!(repo.revoke_token(id1, 30).unwrap());
        assert!(!repo.revoke_token(id1, 31).unwrap());
        assert!(repo.authenticate(&t1).unwrap().is_none());
        assert!(repo.authenticate(&t2).unwrap().is_some());

        repo.set_user_disabled(&u.id, true, 40).unwrap();
        assert!(repo.authenticate(&t2).unwrap().is_none());

        let list = repo.list_tokens(Some(&u.id)).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].revoked_at, Some(30));
        assert_eq!(list[1].id, id2);
        assert_eq!(list[1].user_email, "a@example.com");
        assert!(repo.list_tokens(Some("nobody")).unwrap().is_empty());
        assert_eq!(repo.list_tokens(None).unwrap().len(), 2);
    }

    #[test]
    fn account_limit() {
        let (_d, repo) = repo();
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        for i in 0..MAX_ACCOUNTS_PER_USER {
            repo.ensure_account(&u.id, &format!("{i:08x}"), 1).unwrap();
        }
        repo.ensure_account(&u.id, "00000000", 1).unwrap();
        assert!(matches!(
            repo.ensure_account(&u.id, "ffffffff", 1),
            Err(RepoError::AccountLimit)
        ));
        // Another user is unaffected, and aids are not unique across users.
        let b = repo.create_user(&new_user("b@example.com"), 1).unwrap();
        repo.ensure_account(&b.id, "00000000", 1).unwrap();
    }

    /// Inserts a metadata row directly; the metadata write path arrives in B1-M.
    fn put_metadata(
        repo: &SqliteRepo,
        user: &str,
        aid: &str,
        object_id: &str,
        tracking: Option<&str>,
    ) {
        repo.write(|tx| {
            tx.execute("UPDATE seq SET value = value + 1 WHERE id = 1", [])?;
            tx.execute(
                "INSERT INTO metadata (user_id, aid, object_type, object_id, plugin_id, v, value,
                                       tracking_id, seq, created_at, updated_at)
                 VALUES (?1, ?2, 'message', ?3, 'open-tracking', 1, '{}', ?4,
                         (SELECT value FROM seq WHERE id = 1), 0, 0)",
                params![user, aid, object_id, tracking],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn count(repo: &SqliteRepo, sql: &str) -> i64 {
        repo.read(|c| c.query_row(sql, [], |r| r.get(0))).unwrap()
    }

    fn put_event(repo: &SqliteRepo, user: &str, tracking: &str) {
        repo.write(|tx| {
            tx.execute(
                "INSERT INTO tracking_events (user_id, tracking_id, kind, ts) VALUES (?1, ?2, 'open', 0)",
                params![user, tracking],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn put_claim(repo: &SqliteRepo, user: &str, aid: &str) {
        repo.write(|tx| {
            tx.execute(
                "INSERT INTO claims (user_id, aid, object_id, plugin_id, expiration, created_at)
                 VALUES (?1, ?2, 't:1', 'send-reminders', 5, 0)",
                params![user, aid],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn delete_account_removes_its_rows_only() {
        let (_d, repo) = repo();
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        for aid in ["aaaaaaaa", "bbbbbbbb"] {
            repo.ensure_account(&u.id, aid, 1).unwrap();
            put_claim(&repo, &u.id, aid);
        }
        put_metadata(&repo, &u.id, "aaaaaaaa", "m1", Some("trk-a"));
        put_metadata(&repo, &u.id, "bbbbbbbb", "m2", Some("trk-b"));
        put_event(&repo, &u.id, "trk-a");
        put_event(&repo, &u.id, "trk-b");

        assert!(repo.delete_account(&u.id, "aaaaaaaa").unwrap());
        assert!(!repo.delete_account(&u.id, "aaaaaaaa").unwrap());
        assert_eq!(
            count(
                &repo,
                "SELECT COUNT(*) FROM metadata WHERE aid = 'aaaaaaaa'"
            ),
            0
        );
        assert_eq!(
            count(
                &repo,
                "SELECT COUNT(*) FROM metadata WHERE aid = 'bbbbbbbb'"
            ),
            1
        );
        assert_eq!(count(&repo, "SELECT COUNT(*) FROM claims"), 1);
        assert_eq!(
            count(
                &repo,
                "SELECT COUNT(*) FROM tracking_events WHERE tracking_id = 'trk-a'"
            ),
            0
        );
        assert_eq!(
            count(
                &repo,
                "SELECT COUNT(*) FROM tracking_events WHERE tracking_id = 'trk-b'"
            ),
            1
        );
        assert_eq!(count(&repo, "SELECT COUNT(*) FROM accounts"), 1);
    }

    #[test]
    fn delete_user_removes_everything_they_own() {
        let (_d, repo) = repo();
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        let keep = repo.create_user(&new_user("b@example.com"), 1).unwrap();
        for user in [&u, &keep] {
            repo.create_token(&user.id, &ids::hash_token(&user.id), "t", 1)
                .unwrap();
            repo.ensure_account(&user.id, "aaaaaaaa", 1).unwrap();
            put_metadata(&repo, &user.id, "aaaaaaaa", "m1", Some("trk"));
            put_event(&repo, &user.id, "trk");
            put_event(&repo, &user.id, "pending");
            put_claim(&repo, &user.id, "aaaaaaaa");
        }
        assert!(repo.delete_user(&u.id).unwrap());
        assert!(!repo.delete_user(&u.id).unwrap());
        for table in [
            "tokens",
            "accounts",
            "metadata",
            "tracking_events",
            "claims",
        ] {
            let q = format!("SELECT COUNT(*) FROM {table} WHERE user_id = '{}'", u.id);
            assert_eq!(count(&repo, &q), 0, "{table}");
            let q = format!("SELECT COUNT(*) FROM {table} WHERE user_id = '{}'", keep.id);
            assert!(count(&repo, &q) > 0, "{table} of the other user");
        }
        assert!(repo.user_by_id(&u.id).unwrap().is_none());
    }

    #[test]
    fn upsert_keeps_rowid() {
        // The page order (3.5) depends on this; B1-M's push path must use the same upsert.
        let (_d, repo) = repo();
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        repo.ensure_account(&u.id, "aaaaaaaa", 1).unwrap();
        put_metadata(&repo, &u.id, "aaaaaaaa", "m1", None);
        put_metadata(&repo, &u.id, "aaaaaaaa", "m2", None);
        let id_of = |obj: &str| -> i64 {
            repo.read(|c| {
                c.query_row("SELECT id FROM metadata WHERE object_id = ?1", [obj], |r| {
                    r.get(0)
                })
            })
            .unwrap()
        };
        let before = id_of("m1");
        repo.write(|tx| {
            tx.execute(
                "INSERT INTO metadata (user_id, aid, object_type, object_id, plugin_id, v, value, seq, created_at, updated_at)
                 VALUES (?1, 'aaaaaaaa', 'message', 'm1', 'open-tracking', 2, '[]', 99, 0, 1)
                 ON CONFLICT (user_id, aid, object_type, object_id, plugin_id)
                 DO UPDATE SET v = excluded.v, value = excluded.value, seq = excluded.seq,
                               updated_at = excluded.updated_at",
                [&u.id],
            )?;
            Ok(())
        })
        .unwrap();
        assert_eq!(id_of("m1"), before);
        assert!(id_of("m1") < id_of("m2"));
    }

    #[test]
    fn backup_copies_and_refuses_existing_dest() {
        let (d, repo) = repo();
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        let dest = d.path().join("backup.db");
        let copy_id = repo.backup(&dest).unwrap();
        assert!(repo.backup(&dest).is_err());
        assert!(
            dest.exists(),
            "a refused backup leaves the existing file alone"
        );

        let restored = tempfile::tempdir().unwrap();
        std::fs::copy(&dest, restored.path().join("airpost.db")).unwrap();
        let r = SqliteRepo::open(restored.path()).unwrap();
        assert_eq!(r.user_by_id(&u.id).unwrap(), Some(u));
        assert_eq!(
            r.server_id().unwrap(),
            copy_id,
            "a restore is a new server id"
        );
        assert_ne!(copy_id, repo.server_id().unwrap());

        let rotated = r.rotate_server_id().unwrap();
        assert_ne!(rotated, copy_id);
        assert_eq!(r.server_id().unwrap(), rotated);
    }

    #[cfg(unix)]
    #[test]
    fn files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("data");
        let repo = SqliteRepo::open(&dir).unwrap();
        repo.create_user(&new_user("a@example.com"), 1).unwrap();
        assert_eq!(mode(&dir), 0o700);
        let db = dir.join("airpost.db");
        assert_eq!(mode(&db), 0o600);
        assert_eq!(mode(&dir.join("airpost.db-wal")), 0o600);
        let dest = dir.join("copy.db");
        repo.backup(&dest).unwrap();
        assert_eq!(mode(&dest), 0o600);
        drop(repo);

        // An existing database left world-readable is tightened on open.
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
        drop(SqliteRepo::open(&dir).unwrap());
        assert_eq!(mode(&db), 0o600);
    }

    #[test]
    fn current_seq_counts_every_write() {
        use serde_json::json;
        let (_d, repo) = repo();
        assert_eq!(repo.current_seq().unwrap(), 0);
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        let w = repo
            .push_metadata(&u.id, "aaaaaaaa", &write_of("t:a", None, json!({})), 1)
            .unwrap();
        repo.push_metadata(&u.id, "bbbbbbbb", &write_of("t:a", None, json!({})), 1)
            .unwrap();
        assert_eq!(repo.current_seq().unwrap(), w.seq + 1);
    }

    fn write_of(object_id: &str, version: Option<u64>, value: serde_json::Value) -> MetadataWrite {
        MetadataWrite {
            object_type: crate::wire::ObjectType::Thread,
            object_id: object_id.into(),
            plugin_id: "thread-snooze".into(),
            version,
            value,
            header_message_id: None,
        }
    }

    #[test]
    fn push_follows_the_version_rule_and_keeps_rowids() {
        use serde_json::json;
        let (_d, repo) = repo();
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        let p = |w: &MetadataWrite| repo.push_metadata(&u.id, "aaaaaaaa", w, 5).unwrap();

        let a1 = p(&write_of("t:a", None, json!({"x": 1})));
        assert_eq!(a1.v, 1);
        let b1 = p(&write_of("t:b", Some(4), json!({"x": 2})));
        assert_eq!(b1.v, 4);
        assert!(b1.seq > a1.seq);
        let a2 = p(&write_of("t:a", Some(1), json!({})));
        assert_eq!(a2.v, 2, "pushed version not above stored+1");
        let a3 = p(&write_of("t:a", Some(u64::MAX), json!({"x": 3})));
        assert_eq!(a3.v, crate::wire::V_MAX, "clamped");
        assert_eq!(repo.metadata_head(&u.id, "aaaaaaaa").unwrap(), a3.seq);
        assert_eq!(repo.metadata_head(&u.id, "bbbbbbbb").unwrap(), 0);

        // Pages keep insertion order across updates, and include tombstones.
        let page = repo.metadata_page(&u.id, "aaaaaaaa", 500, 0).unwrap();
        let ids: Vec<_> = page.iter().map(|r| r.object_id.as_str()).collect();
        assert_eq!(ids, ["t:a", "t:b"]);
        p(&write_of("t:a", None, json!([])));
        let page = repo.metadata_page(&u.id, "aaaaaaaa", 500, 0).unwrap();
        assert_eq!(page[0].object_id, "t:a");
        assert_eq!(page[0].value, json!([]));
        assert_eq!(
            repo.metadata_page(&u.id, "aaaaaaaa", 1, 1).unwrap()[0].object_id,
            "t:b"
        );
        assert!(repo
            .metadata_page(&u.id, "aaaaaaaa", 500, 2)
            .unwrap()
            .is_empty());

        // Replay from 0 sees each row once, at its latest seq, in seq order.
        let since = repo.metadata_since(&u.id, "aaaaaaaa", 0, 500).unwrap();
        let ids: Vec<_> = since.iter().map(|(_, r)| r.object_id.as_str()).collect();
        assert_eq!(ids, ["t:b", "t:a"]);
        assert!(since[0].0 < since[1].0);
        assert!(repo
            .metadata_since(&u.id, "aaaaaaaa", since[1].0, 500)
            .unwrap()
            .is_empty());
        assert_eq!(
            repo.metadata_since(&u.id, "aaaaaaaa", 0, 1).unwrap().len(),
            1
        );

        // Another user's account with the same aid is separate.
        let other = repo.create_user(&new_user("b@example.com"), 1).unwrap();
        assert!(repo
            .metadata_page(&other.id, "aaaaaaaa", 500, 0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn header_message_id_keeps_the_last_non_null() {
        use serde_json::json;
        let (_d, repo) = repo();
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        let mut w = write_of("t:a", None, json!({}));
        w.header_message_id = Some("<m@x>".into());
        repo.push_metadata(&u.id, "aaaaaaaa", &w, 1).unwrap();
        w.header_message_id = None;
        repo.push_metadata(&u.id, "aaaaaaaa", &w, 2).unwrap();
        let page = repo.metadata_page(&u.id, "aaaaaaaa", 500, 0).unwrap();
        assert_eq!(page[0].header_message_id.as_deref(), Some("<m@x>"));
    }

    #[test]
    fn tracking_id_is_copied_from_the_value() {
        use serde_json::json;
        let (_d, repo) = repo();
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        let mut w = write_of("m1", None, json!({"trackingId": "trk", "open_count": 0}));
        w.object_type = crate::wire::ObjectType::Message;
        w.plugin_id = "open-tracking".into();
        repo.push_metadata(&u.id, "aaaaaaaa", &w, 1).unwrap();
        assert_eq!(
            count(
                &repo,
                "SELECT COUNT(*) FROM metadata WHERE tracking_id = 'trk'"
            ),
            1
        );
    }

    #[test]
    fn backfill_keeps_stored_values() {
        use serde_json::json;
        let (_d, repo) = repo();
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        let stored = repo
            .push_metadata(
                &u.id,
                "aaaaaaaa",
                &write_of("t:a", Some(3), json!({"s": 1})),
                1,
            )
            .unwrap();
        assert_eq!(stored.v, 3);

        let written = repo
            .backfill_metadata(
                &u.id,
                "aaaaaaaa",
                &[
                    write_of("t:a", Some(2), json!({"local": 1})),
                    write_of("t:new", Some(5), json!({"n": 1})),
                    write_of("t:none", None, json!({"n": 2})),
                ],
                2,
            )
            .unwrap();
        assert_eq!(written.iter().map(|w| w.v).collect::<Vec<_>>(), [5, 1]);
        let rows = repo.metadata_since(&u.id, "aaaaaaaa", 0, 500).unwrap();
        let a = &rows.iter().find(|(_, r)| r.object_id == "t:a").unwrap();
        assert_eq!(
            (a.0, a.1.v, &a.1.value),
            (stored.seq, 3, &json!({"s": 1})),
            "untouched"
        );

        let written = repo
            .backfill_metadata(
                &u.id,
                "aaaaaaaa",
                &[write_of("t:a", Some(3), json!({"local": 2}))],
                3,
            )
            .unwrap();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].v, 4);
        assert!(written[0].seq > stored.seq);
        let rows = repo
            .metadata_since(&u.id, "aaaaaaaa", stored.seq, 500)
            .unwrap();
        let (_, a) = rows.iter().find(|(_, r)| r.object_id == "t:a").unwrap();
        assert_eq!(
            (a.v, &a.value),
            (4, &json!({"s": 1})),
            "server value kept, v raised"
        );
    }

    #[test]
    fn claims() {
        let (_d, repo) = repo();
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        let t1 = repo
            .create_token(&u.id, &ids::hash_token("1"), "a", 1)
            .unwrap();
        let t2 = repo
            .create_token(&u.id, &ids::hash_token("2"), "b", 1)
            .unwrap();
        let claim = |exp: i64, token: i64, now: i64| {
            repo.claim_expiration(&u.id, "aaaaaaaa", "t:1", "send-reminders", exp, token, now)
                .unwrap()
        };
        assert_eq!(claim(100, t1, 10), ClaimOutcome::Granted);
        assert_eq!(claim(100, t2, 11), ClaimOutcome::Taken);
        assert_eq!(claim(100, t1, 12), ClaimOutcome::Granted, "holder retry");
        assert_eq!(claim(200, t2, 13), ClaimOutcome::Granted, "next expiration");

        let other = repo.create_user(&new_user("b@example.com"), 1).unwrap();
        let t3 = repo
            .create_token(&other.id, &ids::hash_token("3"), "c", 1)
            .unwrap();
        assert_eq!(
            repo.claim_expiration(&other.id, "aaaaaaaa", "t:1", "send-reminders", 100, t3, 14)
                .unwrap(),
            ClaimOutcome::Granted
        );

        assert_eq!(repo.purge_claims(13).unwrap(), 1);
        assert_eq!(claim(100, t2, 20), ClaimOutcome::Granted, "purged");
    }

    #[test]
    fn live_token_user() {
        let (_d, repo) = repo();
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        let t = repo
            .create_token(&u.id, &ids::hash_token("1"), "a", 1)
            .unwrap();
        assert_eq!(repo.live_token_user(t).unwrap(), Some(u.clone()));
        repo.set_user_disabled(&u.id, true, 2).unwrap();
        assert_eq!(repo.live_token_user(t).unwrap(), None);
        repo.set_user_disabled(&u.id, false, 3).unwrap();
        repo.revoke_token(t, 4).unwrap();
        assert_eq!(repo.live_token_user(t).unwrap(), None);
        assert_eq!(repo.live_token_user(9999).unwrap(), None);
    }

    #[test]
    fn rowids_survive_backup() {
        use serde_json::json;
        let (d, repo) = repo();
        let u = repo.create_user(&new_user("a@example.com"), 1).unwrap();
        for i in 0..20 {
            repo.push_metadata(
                &u.id,
                "aaaaaaaa",
                &write_of(&format!("t:{i}"), None, json!({})),
                1,
            )
            .unwrap();
        }
        // Holes in the rowid sequence are what VACUUM would renumber without the alias.
        repo.write(|tx| {
            tx.execute(
                "DELETE FROM metadata WHERE object_id IN ('t:3', 't:4', 't:11')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let before = repo.metadata_page(&u.id, "aaaaaaaa", 500, 0).unwrap();
        let dest = d.path().join("backup.db");
        repo.backup(&dest).unwrap();
        let restored = tempfile::tempdir().unwrap();
        std::fs::copy(&dest, restored.path().join("airpost.db")).unwrap();
        let r = SqliteRepo::open(restored.path()).unwrap();
        assert_eq!(r.metadata_page(&u.id, "aaaaaaaa", 500, 0).unwrap(), before);
        let ids = |repo: &SqliteRepo| -> Vec<i64> {
            repo.read(|c| {
                let mut s = c.prepare("SELECT id FROM metadata ORDER BY id")?;
                let rows = s.query_map([], |r| r.get(0))?;
                rows.collect()
            })
            .unwrap()
        };
        assert_eq!(ids(&r), ids(&repo));
    }
}
