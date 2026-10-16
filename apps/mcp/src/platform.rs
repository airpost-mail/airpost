// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Process liveness and owner-only files.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Whether a process with this pid exists. EPERM means it exists under another user, which
/// still counts. Elsewhere every pid counts as alive and a dead engine shows up as a failed
/// connect instead.
#[cfg(unix)]
pub fn pid_alive(pid: u32) -> bool {
    // pid 0 and negative pids address process groups in kill(2).
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 only checks for existence and permission.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
pub fn pid_alive(pid: u32) -> bool {
    pid != 0
}

/// Opens `path` for appending, creating it readable by the owner only.
pub fn open_private_append(path: &Path) -> io::Result<File> {
    let mut o = OpenOptions::new();
    o.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

/// Writes `contents` to a new owner-only file at `path`; fails if it exists.
pub fn create_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut o = OpenOptions::new();
    o.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)?.write_all(contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_pid_is_alive_and_private_files_are_0600() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(0));
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("t");
        create_private(&p, b"x").unwrap();
        assert!(create_private(&p, b"y").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
