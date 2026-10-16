// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Storage (docs/backend-plan.md 3.3, O4). Handlers hold an `Arc<dyn Repo>` and call it from
//! `spawn_blocking`; [`SqliteRepo`] is the only implementation.

pub mod metadata;
mod schema;
mod sqlite;

use std::path::Path;

pub use schema::{MigrateError, SCHEMA_VERSION};
pub use sqlite::SqliteRepo;

use crate::wire::{MetadataRecord, ObjectType};

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("a user with that email address already exists")]
    EmailTaken,
    #[error(
        "account limit reached ({} per user)",
        crate::wire::MAX_ACCOUNTS_PER_USER
    )]
    AccountLimit,
    #[error("{} is in use by a running `tern-backend serve`; stop it first", .0.display())]
    Locked(std::path::PathBuf),
    #[error("{0}")]
    NotRestorable(String),
    #[error(transparent)]
    Migrate(#[from] MigrateError),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = RepoError> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub tracking_ns: String,
    /// Unix milliseconds.
    pub created_at: i64,
    pub disabled_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewUser {
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    /// argon2id PHC string.
    pub password_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenInfo {
    pub id: i64,
    pub user_id: String,
    pub user_email: String,
    pub name: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

/// A live token and its enabled user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authenticated {
    pub user: User,
    pub token_id: i64,
    pub token_last_used_at: Option<i64>,
}

/// One metadata value to store, from a push or a backfill record. Validated by the caller.
#[derive(Debug, Clone, PartialEq)]
pub struct MetadataWrite {
    pub object_type: ObjectType,
    pub object_id: String,
    pub plugin_id: String,
    /// The pushing device's version; `None` and `Some(0)` both mean unknown.
    pub version: Option<u64>,
    pub value: serde_json::Value,
    pub header_message_id: Option<String>,
}

/// The row a write left behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    pub v: u32,
    pub seq: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// The caller holds the claim: it was free, or this token took it before.
    Granted,
    /// Another device holds it.
    Taken,
}

/// Emails are compared trimmed and lowercased.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

pub trait Repo: Send + Sync + 'static {
    fn schema_version(&self) -> Result<u32>;
    /// Random per instance, created by the first migration run.
    fn server_id(&self) -> Result<String>;
    /// Replaces the server id, so engines treat the next pull as a first pull (4.1). For a
    /// database restored from a copy, whose seq counter went back. Returns the new id.
    fn rotate_server_id(&self) -> Result<String>;
    /// The global seq counter: the largest seq ever handed out.
    fn current_seq(&self) -> Result<i64>;

    fn create_user(&self, new: &NewUser, now: i64) -> Result<User>;
    fn user_by_id(&self, id: &str) -> Result<Option<User>>;
    fn user_by_email(&self, email: &str) -> Result<Option<User>>;
    /// The user and its password hash, for login.
    fn login_candidate(&self, email: &str) -> Result<Option<(User, String)>>;
    fn list_users(&self) -> Result<Vec<User>>;
    /// Returns false when the user does not exist.
    fn set_user_disabled(&self, user_id: &str, disabled: bool, now: i64) -> Result<bool>;
    /// Removes the user and every row they own. Returns false when the user does not exist.
    fn delete_user(&self, user_id: &str) -> Result<bool>;

    fn create_token(&self, user_id: &str, hash: &[u8; 32], name: &str, now: i64) -> Result<i64>;
    /// Finds a token that is not revoked and whose user is not disabled.
    fn authenticate(&self, hash: &[u8; 32]) -> Result<Option<Authenticated>>;
    fn touch_token(&self, token_id: i64, now: i64) -> Result<()>;
    fn list_tokens(&self, user_id: Option<&str>) -> Result<Vec<TokenInfo>>;
    /// Returns false when the token does not exist or was already revoked.
    fn revoke_token(&self, token_id: i64, now: i64) -> Result<bool>;

    /// Creates `(user, aid)` on first use; fails with [`RepoError::AccountLimit`] past 50.
    fn ensure_account(&self, user_id: &str, aid: &str, now: i64) -> Result<()>;
    /// Deletes the account with its metadata, claims, and the tracking events of its tracked
    /// messages. Returns false when it did not exist.
    fn delete_account(&self, user_id: &str, aid: &str) -> Result<bool>;

    /// Stores a push under the version rule (3.5), creating the account on first use.
    fn push_metadata(
        &self,
        user_id: &str,
        aid: &str,
        w: &MetadataWrite,
        now: i64,
    ) -> Result<Written>;
    /// Applies a backfill batch in one transaction (3.5). Returns the rows it changed, in
    /// seq order.
    fn backfill_metadata(
        &self,
        user_id: &str,
        aid: &str,
        records: &[MetadataWrite],
        now: i64,
    ) -> Result<Vec<Written>>;
    /// The largest seq of the account, or 0.
    fn metadata_head(&self, user_id: &str, aid: &str) -> Result<i64>;
    /// Rows of the account in rowid order, tombstones included.
    fn metadata_page(
        &self,
        user_id: &str,
        aid: &str,
        limit: usize,
        offset: u64,
    ) -> Result<Vec<MetadataRecord>>;
    /// Rows of the account with `seq > after`, in seq order, with their seq.
    fn metadata_since(
        &self,
        user_id: &str,
        aid: &str,
        after: i64,
        limit: usize,
    ) -> Result<Vec<(i64, MetadataRecord)>>;

    /// Takes the expiration claim for `token_id` (3.8), creating the account on first use.
    #[allow(clippy::too_many_arguments)]
    fn claim_expiration(
        &self,
        user_id: &str,
        aid: &str,
        object_id: &str,
        plugin_id: &str,
        expiration: i64,
        token_id: i64,
        now: i64,
    ) -> Result<ClaimOutcome>;
    /// Deletes claims created before `before`. Returns how many.
    fn purge_claims(&self, before: i64) -> Result<usize>;

    /// The user of a token that is still live (not revoked, user enabled and present).
    fn live_token_user(&self, token_id: i64) -> Result<Option<User>>;

    /// A consistent copy of the database at `dest` (`VACUUM INTO`), readable by the owner
    /// only, with a new server id so that restoring it makes engines re-sync instead of
    /// skipping writes past the restored seq. `dest` must not exist. Returns the copy's id.
    fn backup(&self, dest: &Path) -> Result<String>;
}
