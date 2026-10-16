// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Scheduled backups taken by `serve` (docs/deployment-plan.md 3.5): `tern-<time>.db` in the
//! configured directory every interval, keeping the newest `keep`. Each is a
//! [`Repo::backup`], so it has a new server id and restores like a manual backup.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::config::BackupSchedule;
use crate::ids;
use crate::repo::Repo;

const PREFIX: &str = "tern-";
const SUFFIX: &str = ".db";

type BoxError = Box<dyn Error + Send + Sync>;

/// `tern-YYYYMMDDTHHMMSSZ.db`. Only names this module writes are listed or pruned, so other
/// files in the directory are never deleted.
fn is_backup_name(name: &str) -> bool {
    let Some(t) = name
        .strip_prefix(PREFIX)
        .and_then(|r| r.strip_suffix(SUFFIX))
    else {
        return false;
    };
    t.len() == 16
        && t.char_indices().all(|(i, c)| match i {
            8 => c == 'T',
            15 => c == 'Z',
            _ => c.is_ascii_digit(),
        })
}

/// The scheduled backups in `dir`, oldest first.
pub fn list(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(is_backup_name)
        })
        .collect();
    found.sort();
    Ok(found)
}

/// Writes one backup named for `now_ms`, then deletes the oldest beyond `schedule.keep`.
pub fn take(repo: &dyn Repo, schedule: &BackupSchedule, now_ms: i64) -> Result<PathBuf, BoxError> {
    if !schedule.dir.is_dir() {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(&schedule.dir)?;
    }
    let path = schedule
        .dir
        .join(format!("{PREFIX}{}{SUFFIX}", ids::file_time(now_ms)));
    repo.backup(&path)?;
    let all = list(&schedule.dir)?;
    for old in &all[..all.len().saturating_sub(schedule.keep)] {
        if let Err(e) = std::fs::remove_file(old) {
            tracing::warn!(path = %old.display(), error = %e, "could not delete an old backup");
        }
    }
    Ok(path)
}

/// Time until the next backup: `interval` after the newest existing one, or now.
pub fn first_delay(schedule: &BackupSchedule, now: SystemTime) -> Duration {
    let newest = list(&schedule.dir)
        .ok()
        .and_then(|all| all.last().cloned())
        .and_then(|p| p.metadata().ok())
        .and_then(|m| m.modified().ok());
    match newest {
        Some(t) => schedule
            .interval
            .saturating_sub(now.duration_since(t).unwrap_or_default()),
        None => Duration::ZERO,
    }
}

/// Takes backups on `schedule` forever. Failures are logged and retried at the next
/// interval; they never stop the server.
pub async fn run(repo: Arc<dyn Repo>, schedule: BackupSchedule) {
    let mut delay = first_delay(&schedule, SystemTime::now());
    loop {
        tokio::time::sleep(delay).await;
        delay = schedule.interval;
        let (r, s) = (repo.clone(), schedule.clone());
        match tokio::task::spawn_blocking(move || take(&*r, &s, ids::now_ms())).await {
            Ok(Ok(path)) => tracing::info!(path = %path.display(), "backup written"),
            Ok(Err(e)) => {
                tracing::error!(dir = %schedule.dir.display(), error = %e, "backup failed")
            }
            Err(e) => tracing::error!(error = %e, "backup task failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::SqliteRepo;

    #[test]
    fn names() {
        assert!(is_backup_name("tern-20261016T120000Z.db"));
        for bad in [
            "tern.db",
            "tern-20261016T120000Z.db-wal",
            "tern-20261016-120000Z.db",
            "tern-2026101xT120000Z.db",
            "pre-migrate-1-to-2-20261016T120000Z.db",
            "tern-20261016T120000.db",
        ] {
            assert!(!is_backup_name(bad), "{bad}");
        }
    }

    #[test]
    fn takes_and_prunes() {
        let data = tempfile::tempdir().unwrap();
        let repo = SqliteRepo::open(data.path()).unwrap();
        let out = tempfile::tempdir().unwrap();
        let schedule = BackupSchedule {
            dir: out.path().join("backups"),
            interval: Duration::from_secs(3600),
            keep: 2,
        };
        assert_eq!(first_delay(&schedule, SystemTime::now()), Duration::ZERO);

        std::fs::create_dir(&schedule.dir).unwrap();
        let unrelated = schedule.dir.join("notes.txt");
        std::fs::write(&unrelated, "keep me").unwrap();
        let base = 1_760_000_000_000;
        for i in 0..4 {
            take(&repo, &schedule, base + i * 60_000).unwrap();
        }
        let left = list(&schedule.dir).unwrap();
        let names: Vec<_> = left
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            names,
            [
                format!("tern-{}.db", ids::file_time(base + 120_000)),
                format!("tern-{}.db", ids::file_time(base + 180_000)),
            ]
        );
        assert!(unrelated.exists());
        // A backup is a database with its own server id.
        let copy = SqliteRepo::open(&schedule.dir.join("restore-test")).unwrap();
        drop(copy);
        std::fs::copy(&left[1], schedule.dir.join("restore-test/tern.db")).unwrap();
        let restored = SqliteRepo::open(&schedule.dir.join("restore-test")).unwrap();
        assert_ne!(restored.server_id().unwrap(), repo.server_id().unwrap());

        // The newest backup was just written, so the next one is a full interval away.
        let d = first_delay(&schedule, SystemTime::now());
        assert!(d > Duration::from_secs(3500), "{d:?}");
        let later = SystemTime::now() + Duration::from_secs(4000);
        assert_eq!(first_delay(&schedule, later), Duration::ZERO);
    }

    #[test]
    fn existing_name_fails_without_pruning() {
        let data = tempfile::tempdir().unwrap();
        let repo = SqliteRepo::open(data.path()).unwrap();
        let out = tempfile::tempdir().unwrap();
        let schedule = BackupSchedule {
            dir: out.path().to_owned(),
            interval: Duration::from_secs(3600),
            keep: 1,
        };
        take(&repo, &schedule, 1_760_000_000_000).unwrap();
        assert!(take(&repo, &schedule, 1_760_000_000_000).is_err());
        assert_eq!(list(out.path()).unwrap().len(), 1);
    }
}
