// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Process liveness, for telling a live discovery file from one a crash left behind.

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
}
