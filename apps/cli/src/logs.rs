// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern logs`: the engine's per-account log, `<config-dir>/mailsync-<accountId>.log`. The
//! engine rotates it at 5 MB by renaming it to `.log.1` (and `.1` to `.2`, up to `.3`) and
//! starting a new file (`tern_core::log`), so following it means noticing the rename.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use crate::error::{CliError, Result};
use crate::platform;

const POLL: Duration = Duration::from_millis(250);

pub fn log_path(config_dir: &Path, account_id: &str) -> PathBuf {
    config_dir.join(format!("mailsync-{account_id}.log"))
}

fn rotated(base: &Path, index: usize) -> PathBuf {
    let mut s = base.as_os_str().to_owned();
    s.push(format!(".{index}"));
    PathBuf::from(s)
}

/// The account ids that have a log in `config_dir`.
pub fn logged_accounts(config_dir: &Path) -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(config_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| {
            n.strip_prefix("mailsync-")?
                .strip_suffix(".log")
                .map(str::to_owned)
        })
        .collect();
    ids.sort();
    ids
}

/// The last `n` lines of the log, reaching into `.log.1` when the current file is shorter,
/// and a follower that starts right after what was read, on the same open file, so nothing
/// is printed twice or missed. An unterminated last line is one of the `n` when
/// `keep_partial`; otherwise the follower prints it once the engine ends it.
pub fn tail(base: &Path, n: usize, keep_partial: bool) -> io::Result<(Vec<String>, Follower)> {
    let mut file = File::open(base)?;
    let id = platform::file_id(&file.metadata()?);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let pos = bytes.len() as u64;
    let complete = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let mut partial = bytes.split_off(complete);
    let mut lines = to_lines(&bytes);
    if keep_partial && !partial.is_empty() {
        lines.push(String::from_utf8_lossy(&std::mem::take(&mut partial)).into_owned());
    }
    if lines.len() < n {
        if let Ok(older) = std::fs::read(rotated(base, 1)) {
            let mut older = to_lines(&older);
            older.append(&mut lines);
            lines = older;
        }
    }
    let skip = lines.len().saturating_sub(n);
    let follower = Follower {
        path: base.to_owned(),
        file: Some(file),
        id,
        pos,
        partial,
    };
    Ok((lines.split_off(skip), follower))
}

fn to_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Reads what is appended to a file, and moves to the new file when the path is rotated
/// (a new file id) or truncated (shorter than what was read).
pub struct Follower {
    path: PathBuf,
    file: Option<File>,
    id: Option<(u64, u64)>,
    pos: u64,
    partial: Vec<u8>,
}

impl Follower {
    /// Starts at the end of the file, or at its start once it appears.
    #[cfg(test)]
    pub fn at_end(path: PathBuf) -> Self {
        let mut f = Follower {
            path,
            file: None,
            id: None,
            pos: 0,
            partial: Vec::new(),
        };
        if let Ok(file) = File::open(&f.path) {
            if let Ok(meta) = file.metadata() {
                f.id = platform::file_id(&meta);
                f.pos = meta.len();
            }
            f.file = Some(file);
        }
        f
    }

    /// Complete lines written since the last poll.
    pub fn poll(&mut self) -> io::Result<Vec<String>> {
        let mut out = Vec::new();
        let current = std::fs::metadata(&self.path).ok();
        let current_id = current.as_ref().and_then(platform::file_id);
        let rotated = match (&current, self.file.is_some()) {
            (None, _) => false,
            (Some(_), false) => true,
            (Some(meta), true) => {
                (current_id.is_some() && current_id != self.id) || meta.len() < self.pos
            }
        };
        if rotated {
            // Lines the engine wrote to the old file just before renaming it.
            self.read_into(&mut out)?;
            self.flush_partial(&mut out);
            self.file = File::open(&self.path).ok();
            self.id = current_id;
            self.pos = 0;
        }
        self.read_into(&mut out)?;
        Ok(out)
    }

    fn read_into(&mut self, out: &mut Vec<String>) -> io::Result<()> {
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        file.seek(SeekFrom::Start(self.pos))?;
        let mut buf = Vec::new();
        self.pos += file.read_to_end(&mut buf)? as u64;
        self.partial.extend_from_slice(&buf);
        while let Some(nl) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=nl).collect();
            let text = String::from_utf8_lossy(&line[..nl]);
            out.push(text.trim_end_matches('\r').to_owned());
        }
        Ok(())
    }

    fn flush_partial(&mut self, out: &mut Vec<String>) {
        if !self.partial.is_empty() {
            out.push(String::from_utf8_lossy(&std::mem::take(&mut self.partial)).into_owned());
        }
    }
}

/// The log file for `account`: an id with a log, or an email a running engine maps to one.
pub fn resolve(config_dir: &Path, account: &str) -> Result<PathBuf> {
    let direct = log_path(config_dir, account);
    if direct.is_file() {
        return Ok(direct);
    }
    // Best effort: an email address needs a running engine to name its account id.
    if let Ok(engines) = crate::discovery::connect_all_quiet(config_dir) {
        for e in &engines {
            for a in &e.hello.accounts {
                if crate::discovery::matches(a, account) {
                    let p = log_path(config_dir, &a.account_id);
                    if p.is_file() {
                        return Ok(p);
                    }
                }
            }
        }
    }
    let known = logged_accounts(config_dir);
    let hint = if known.is_empty() {
        format!(
            "there are no engine logs in {}; check --config-dir",
            config_dir.display()
        )
    } else {
        format!(
            "give an account id (logs exist for {}), or the email of an account whose engine is \
             running",
            known.join(", ")
        )
    };
    Err(CliError::failed(format!("no log for {account}: {hint}")))
}

pub fn run(
    config_dir: &Path,
    account: &str,
    follow: bool,
    lines: usize,
    json_out: bool,
) -> Result<()> {
    let path = resolve(config_dir, account)?;
    let emit = |l: &str| {
        if json_out {
            crate::output::json(&json!({ "line": l }));
        } else {
            crate::output::line(l);
        }
    };
    let (first, mut follower) = tail(&path, lines, !follow)
        .map_err(|e| CliError::failed(format!("could not read {}: {e}", path.display())))?;
    for l in &first {
        emit(l);
    }
    if !follow {
        return Ok(());
    }
    loop {
        match follower.poll() {
            Ok(new) => new.iter().for_each(|l| emit(l)),
            Err(e) => {
                return Err(CliError::failed(format!(
                    "could not read {}: {e}",
                    path.display()
                )))
            }
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn append(path: &Path, s: &str) {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(s.as_bytes())
            .unwrap();
    }

    #[test]
    fn tail_reaches_into_the_rotated_file() {
        let tmp = tempfile::tempdir().unwrap();
        let base = log_path(tmp.path(), "acc");
        append(&rotated(&base, 1), "old1\nold2\nold3\n");
        append(&base, "new1\nnew2\n");
        let lines = |n| tail(&base, n, true).unwrap().0;
        assert_eq!(lines(1), ["new2"]);
        assert_eq!(lines(2), ["new1", "new2"]);
        assert_eq!(lines(4), ["old2", "old3", "new1", "new2"]);
        assert_eq!(logged_accounts(tmp.path()), ["acc"]);
    }

    #[test]
    fn follow_starts_where_the_tail_ended() {
        let tmp = tempfile::tempdir().unwrap();
        let base = log_path(tmp.path(), "acc");
        append(&base, "one\ntwo\nhal");
        assert_eq!(tail(&base, 5, true).unwrap().0, ["one", "two", "hal"]);
        let (first, mut f) = tail(&base, 5, false).unwrap();
        assert_eq!(first, ["one", "two"]);
        append(&base, "f\nthree\n");
        assert_eq!(f.poll().unwrap(), ["half", "three"]);
        assert!(f.poll().unwrap().is_empty());
    }

    #[test]
    fn follower_survives_rotation_and_partial_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let base = log_path(tmp.path(), "acc");
        append(&base, "before\n");
        let mut f = Follower::at_end(base.clone());
        assert!(f.poll().unwrap().is_empty());

        append(&base, "one\ntw");
        assert_eq!(f.poll().unwrap(), ["one"]);
        append(&base, "o\n");
        assert_eq!(f.poll().unwrap(), ["two"]);

        // The engine's rotation: a last line into the old file, rename, a new file.
        append(&base, "last in old\n");
        std::fs::rename(&base, rotated(&base, 1)).unwrap();
        append(&base, "first in new\n");
        let got = f.poll().unwrap();
        if cfg!(unix) {
            assert_eq!(got, ["last in old", "first in new"]);
        }
        append(&base, "second in new\n");
        assert_eq!(f.poll().unwrap(), ["second in new"]);

        // Truncated in place.
        std::fs::write(&base, "").unwrap();
        append(&base, "after truncate\n");
        assert_eq!(f.poll().unwrap(), ["after truncate"]);
    }

    #[test]
    fn follower_waits_for_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let base = log_path(tmp.path(), "acc");
        let mut f = Follower::at_end(base.clone());
        assert!(f.poll().unwrap().is_empty());
        append(&base, "hello\n");
        assert_eq!(f.poll().unwrap(), ["hello"]);
    }

    #[test]
    fn unknown_account_lists_the_logged_ones() {
        let tmp = tempfile::tempdir().unwrap();
        append(&log_path(tmp.path(), "acc-1"), "x\n");
        let e = resolve(tmp.path(), "nobody@x.test").unwrap_err();
        assert_eq!(e.exit_code(), 1);
        assert!(e.to_string().contains("acc-1"), "{e}");
        assert_eq!(
            resolve(tmp.path(), "acc-1").unwrap(),
            log_path(tmp.path(), "acc-1")
        );
    }
}
