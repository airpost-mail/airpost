// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Process and socket liveness, for telling a live discovery file from one a crash left
//! behind.

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
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
pub fn pid_alive(pid: u32) -> bool {
    pid != 0
}

/// Whether something accepts connections on this Unix socket. An engine in another pid
/// namespace (a container sharing the profile volume) has a pid this process can't see, but
/// its socket still answers.
#[cfg(unix)]
pub fn socket_listening(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

#[cfg(not(unix))]
pub fn socket_listening(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_pid_is_alive_and_reaped_child_is_not() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(0));
        #[cfg(unix)]
        {
            assert!(!pid_alive(u32::MAX));
            let mut child = std::process::Command::new("true").spawn().unwrap();
            let pid = child.id();
            child.wait().unwrap();
            assert!(!pid_alive(pid));
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_bound_socket_listens_and_a_removed_one_does_not() {
        // Removed rather than only closed: a child another test forks may still hold the fd.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("e.sock");
        assert!(!socket_listening(&path));
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(socket_listening(&path));
        drop(listener);
        std::fs::remove_file(&path).unwrap();
        assert!(!socket_listening(&path));
    }
}
