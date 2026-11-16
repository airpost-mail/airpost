// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! What a Airpost program run by a user unit needs from its service manager.
//!
//! `airpost gateway install` and `airpost mcp-http install` write units whose manager holds the TCP
//! socket (launchd `Sockets`, a systemd `.socket` unit) and starts the program on the first
//! connection. [`tcp_listener`] takes that socket: launchd's through
//! `launch_activate_socket(3)`, systemd's through the `LISTEN_FDS` protocol
//! (`sd_listen_fds(3)`). Started by hand there is none, and the program binds its own.
//!
//! [`build_id`] names the binary a process runs, as the engine's `hello.build` does, so
//! `airpost status` can tell when a running service is another build than the one installed.

use std::io::{self, Read};
use std::net::TcpListener;
use std::path::Path;

use sha2::{Digest, Sha256};

/// The listening TCP socket the service manager passed under `name` (the key in the launchd
/// job's `Sockets`, the systemd socket unit's `FileDescriptorName=`), or None when the process
/// wasn't started with one. Call it before spawning threads: on Linux it unsets the
/// `LISTEN_*` variables, so the program's children don't see the socket as theirs.
pub fn tcp_listener(name: &str) -> io::Result<Option<TcpListener>> {
    imp::take(name)
}

/// `sha256:<hex>` of this program's binary.
pub fn build_id() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    file_build_id(&exe).ok()
}

/// `sha256:<hex>` of the file at `path`.
pub fn file_build_id(path: &Path) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(format!("sha256:{hex}"))
}

/// Which of the `count` passed descriptors is `want`: by `LISTEN_FDNAMES` (colon-separated),
/// else the only one.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn pick(names: Option<&str>, want: &str, count: usize) -> Option<usize> {
    if let Some(i) = names.and_then(|n| n.split(':').position(|n| n == want)) {
        return (i < count).then_some(i);
    }
    (count == 1).then_some(0)
}

/// A listening stream socket with an inet address, as a [`TcpListener`] that isn't inherited
/// by the program's children.
#[cfg(unix)]
fn listener_from_fd(fd: std::os::fd::RawFd) -> io::Result<TcpListener> {
    use std::os::fd::FromRawFd;
    let int_opt = |opt: libc::c_int| -> io::Result<libc::c_int> {
        let mut value: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: value and len are valid for writes of the sizes passed.
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                opt,
                (&mut value as *mut libc::c_int).cast(),
                &mut len,
            )
        };
        if rc == 0 {
            Ok(value)
        } else {
            Err(io::Error::last_os_error())
        }
    };
    let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, what.to_owned());
    if int_opt(libc::SO_TYPE)? != libc::SOCK_STREAM {
        return Err(invalid("the passed socket is not a stream socket"));
    }
    // macOS answers ENOPROTOOPT for SO_ACCEPTCONN; launchd only passes listening sockets.
    match int_opt(libc::SO_ACCEPTCONN) {
        Ok(0) => return Err(invalid("the passed socket is not listening")),
        Err(e) if e.raw_os_error() != Some(libc::ENOPROTOOPT) => return Err(e),
        _ => {}
    }
    // SAFETY: fcntl on a descriptor this process holds; FD_CLOEXEC only affects exec.
    unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    // SAFETY: the manager handed this descriptor to this process, and nothing else owns it.
    let listener = unsafe { TcpListener::from_raw_fd(fd) };
    // A Unix socket has no inet address and fails here.
    listener
        .local_addr()
        .map_err(|_| invalid("the passed socket is not a TCP socket"))?;
    Ok(listener)
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;

    /// `SD_LISTEN_FDS_START`.
    const FIRST_FD: i32 = 3;
    const VARS: [&str; 3] = ["LISTEN_PID", "LISTEN_FDS", "LISTEN_FDNAMES"];

    pub fn take(name: &str) -> io::Result<Option<TcpListener>> {
        let [pid, fds, names] = VARS.map(|k| std::env::var(k).ok());
        for k in VARS {
            std::env::remove_var(k);
        }
        let (Some(pid), Some(fds)) = (pid, fds) else {
            return Ok(None);
        };
        // Set for another process: inherited through a fork that didn't unset them.
        if pid.trim().parse::<u32>().ok() != Some(std::process::id()) {
            return Ok(None);
        }
        let count: usize = fds.trim().parse().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "LISTEN_FDS is not a number")
        })?;
        let Some(index) = pick(names.as_deref(), name, count) else {
            return Ok(None);
        };
        listener_from_fd(FIRST_FD + index as i32).map(Some)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::ffi::CString;

    extern "C" {
        /// `<launch.h>`, macOS 10.10 and later. Returns 0 or an errno value; `fds` is
        /// malloc'd and the caller frees it.
        fn launch_activate_socket(
            name: *const libc::c_char,
            fds: *mut *mut libc::c_int,
            cnt: *mut libc::size_t,
        ) -> libc::c_int;
    }

    pub fn take(name: &str) -> io::Result<Option<TcpListener>> {
        let name = CString::new(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a NUL in the name"))?;
        let mut fds: *mut libc::c_int = std::ptr::null_mut();
        let mut cnt: libc::size_t = 0;
        // SAFETY: name is NUL-terminated; fds and cnt are valid for writes.
        let rc = unsafe { launch_activate_socket(name.as_ptr(), &mut fds, &mut cnt) };
        match rc {
            0 => {}
            // ESRCH: not a launchd job; ENOENT: a job without a socket of this name.
            libc::ESRCH | libc::ENOENT => return Ok(None),
            e => return Err(io::Error::from_raw_os_error(e)),
        }
        let all: Vec<libc::c_int> = if fds.is_null() {
            Vec::new()
        } else {
            // SAFETY: launchd returned an array of cnt descriptors at fds.
            let v = unsafe { std::slice::from_raw_parts(fds, cnt) }.to_vec();
            // SAFETY: fds was malloc'd by launch_activate_socket and is freed once.
            unsafe { libc::free(fds.cast()) };
            v
        };
        let mut all = all.into_iter();
        let Some(first) = all.next() else {
            return Ok(None);
        };
        // The units name one numeric address and family, so there is one descriptor; any other
        // would only hold the port open.
        for extra in all {
            // SAFETY: a descriptor launchd handed to this process and nothing else uses.
            unsafe { libc::close(extra) };
        }
        listener_from_fd(first).map(Some)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod imp {
    use super::*;

    pub fn take(_name: &str) -> io::Result<Option<TcpListener>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_by_name_or_the_only_one() {
        assert_eq!(pick(Some("http:control"), "control", 2), Some(1));
        assert_eq!(pick(Some("http"), "http", 1), Some(0));
        assert_eq!(pick(None, "http", 1), Some(0));
        assert_eq!(pick(None, "http", 2), None);
        assert_eq!(pick(Some("a:b"), "http", 2), None);
        assert_eq!(pick(Some("other"), "http", 1), Some(0));
        assert_eq!(pick(Some("x:http"), "http", 1), None);
    }

    #[test]
    fn no_manager_socket_outside_a_unit() {
        assert!(tcp_listener("airpost-activation-test").unwrap().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn takes_listening_tcp_sockets_only() {
        use std::os::fd::IntoRawFd;
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = tcp.local_addr().unwrap();
        let taken = listener_from_fd(tcp.into_raw_fd()).unwrap();
        assert_eq!(taken.local_addr().unwrap(), addr);
        assert!(std::net::TcpStream::connect(addr).is_ok());

        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        assert!(listener_from_fd(udp.into_raw_fd()).is_err());
        let tmp =
            std::env::temp_dir().join(format!("airpost-activation-{}.sock", std::process::id()));
        let unix = std::os::unix::net::UnixListener::bind(&tmp).unwrap();
        let e = listener_from_fd(unix.into_raw_fd()).unwrap_err();
        let _ = std::fs::remove_file(&tmp);
        assert!(e.to_string().contains("TCP"), "{e}");
    }

    #[test]
    fn build_ids_are_the_files_sha256() {
        let tmp =
            std::env::temp_dir().join(format!("airpost-activation-{}.bin", std::process::id()));
        std::fs::write(&tmp, b"abc").unwrap();
        let id = file_build_id(&tmp).unwrap();
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(
            id,
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(build_id().unwrap().starts_with("sha256:"));
    }
}
