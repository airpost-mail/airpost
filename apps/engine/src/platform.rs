// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The few things that differ by OS. Nothing outside this module uses
//! Unix-only APIs.

/// `test-segfault`: crash the way a real segfault would, for the client's crash handling.
pub fn raise_segfault() -> ! {
    #[cfg(unix)]
    // SAFETY: raise only delivers a signal to this process; SIGSEGV's default action
    // terminates it.
    unsafe {
        libc::raise(libc::SIGSEGV);
    }
    std::process::abort()
}

/// Makes every file and directory the engine creates private to its user (TD12): the
/// database, logs, attachments and sockets hold mail.
pub fn restrict_umask() {
    #[cfg(unix)]
    // SAFETY: umask only changes this process's file mode mask.
    unsafe {
        libc::umask(0o077);
    }
}

/// Exits without running destructors or flushing deltas, as `std::exit(141)` did. Files
/// registered with [`remove_on_exit`] are removed first.
pub fn exit_now(code: i32) -> ! {
    airpost_core::log::flush();
    remove_registered_files();
    std::process::exit(code)
}

static REMOVE_ON_EXIT: std::sync::Mutex<Vec<std::path::PathBuf>> =
    std::sync::Mutex::new(Vec::new());

/// Removes `path` when the process exits through [`exit_now`] or, after
/// [`remove_on_termination`], SIGTERM or SIGINT (the socket and its discovery file). An abort
/// or SIGKILL leaves it; the next engine replaces a stale socket, and clients skip a
/// discovery file whose pid is gone.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn remove_on_exit(path: std::path::PathBuf) {
    REMOVE_ON_EXIT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(path);
}

fn remove_registered_files() {
    let files = std::mem::take(&mut *REMOVE_ON_EXIT.lock().unwrap_or_else(|e| e.into_inner()));
    for f in files {
        let _ = std::fs::remove_file(f);
    }
}

/// The write end of the self-pipe the SIGTERM / SIGINT handler writes to.
#[cfg(unix)]
static SIGNAL_PIPE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

#[cfg(unix)]
extern "C" fn on_termination_signal(sig: libc::c_int) {
    let fd = SIGNAL_PIPE.load(std::sync::atomic::Ordering::Relaxed);
    let byte = sig as u8;
    // SAFETY: write is async-signal-safe and byte outlives the call. The pipe gets one byte
    // per signal and is read at once, so the write cannot fail and leave errno changed.
    unsafe {
        libc::write(fd, (&byte as *const u8).cast(), 1);
    }
}

/// On SIGTERM (how the desktop stops an engine) or SIGINT, removes the files registered with
/// [`remove_on_exit`] and then dies of the same signal, so the parent sees what it saw
/// before. The handler only writes to a pipe; a watcher thread does the rest.
#[cfg(unix)]
pub fn remove_on_termination() -> std::io::Result<()> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: fds has room for the two descriptors pipe writes.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let [read_fd, write_fd] = fds;
    for fd in fds {
        // SAFETY: fd was just opened by pipe; FD_CLOEXEC keeps it out of child processes.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    SIGNAL_PIPE.store(write_fd, std::sync::atomic::Ordering::Relaxed);
    std::thread::Builder::new()
        .name(airpost_core::constants::thread::SIGNALS.into())
        .spawn(move || watch_termination(read_fd))?;
    for sig in [libc::SIGTERM, libc::SIGINT] {
        // SAFETY: the handler only loads an atomic and calls write. SA_RESTART keeps the
        // signal from failing other threads' blocking calls with EINTR.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_termination_signal as *const () as libc::sighandler_t;
            action.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            if libc::sigaction(sig, &action, std::ptr::null_mut()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn watch_termination(read_fd: libc::c_int) {
    let mut byte = 0u8;
    loop {
        // SAFETY: byte is valid for a one-byte write; read_fd stays open for the process.
        let n = unsafe { libc::read(read_fd, (&mut byte as *mut u8).cast(), 1) };
        if n == 1 {
            break;
        }
        if n == 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return;
        }
    }
    let sig = libc::c_int::from(byte);
    tracing::info!("Received signal {sig}; removing the socket files and exiting");
    remove_registered_files();
    airpost_core::log::flush();
    // SAFETY: restoring the default action and re-raising ends the process by `sig`.
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
    std::process::exit(128 + sig)
}

/// The Unix domain socket transport's OS calls.
#[cfg(unix)]
pub mod uds {
    use std::fs;
    use std::io;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;

    /// `sizeof(sockaddr_un.sun_path)`, which includes the terminating NUL.
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    const SUN_PATH_LEN: usize = 104;
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )))]
    const SUN_PATH_LEN: usize = 108;

    /// Creates `dir` and any missing parents as `0700`. Existing directories keep their mode.
    pub fn create_private_dir(dir: &Path) -> io::Result<()> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }

    /// Creates `dir` as [`create_private_dir`] does and closes an existing one to group and
    /// others: everything the engine keeps in it is the user's mail.
    pub fn restrict_dir(dir: &Path) -> io::Result<()> {
        create_private_dir(dir)?;
        let mode = fs::metadata(dir)?.permissions().mode();
        if mode & 0o077 != 0 {
            fs::set_permissions(dir, fs::Permissions::from_mode(mode & 0o700))?;
        }
        Ok(())
    }

    /// Writes `bytes` to `path` as a `0600` file, replacing it atomically.
    pub fn write_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, bytes)?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
        fs::rename(&tmp, path)
    }

    /// Why [`bind`] failed: `message` is the one-shot message for the client, `in_use` that
    /// another process answers on the socket.
    pub struct BindError {
        pub message: String,
        pub in_use: bool,
    }

    impl From<String> for BindError {
        fn from(message: String) -> Self {
            BindError {
                message,
                in_use: false,
            }
        }
    }

    /// Binds a `0600` socket at `path`, creating its parent directory as `0700` when it is
    /// missing. An existing socket file is replaced only when nothing answers on it.
    pub fn bind(path: &Path) -> Result<UnixListener, BindError> {
        let shown = path.display();
        let len = path.as_os_str().len();
        if len >= SUN_PATH_LEN {
            return Err(format!(
                "Socket path {shown} is {len} bytes; this OS allows {}. Choose a shorter path.",
                SUN_PATH_LEN - 1
            )
            .into());
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            create_private_dir(parent)
                .map_err(|e| format!("Could not create the socket directory: {e}"))?;
        }
        match fs::symlink_metadata(path) {
            Ok(meta) if !meta.file_type().is_socket() => {
                return Err(format!("{shown} exists and is not a socket").into());
            }
            Ok(_) => {
                if UnixStream::connect(path).is_ok() {
                    return Err(BindError {
                        message: format!("Another process is listening on {shown}"),
                        in_use: true,
                    });
                }
                fs::remove_file(path)
                    .map_err(|e| format!("Could not remove the stale socket {shown}: {e}"))?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Could not check {shown}: {e}").into()),
        }
        // The umask keeps the socket private between bind and chmod. No other thread
        // creates files this early in the process.
        // SAFETY: umask only swaps the process's file mode mask.
        let old = unsafe { libc::umask(0o177) };
        let bound = UnixListener::bind(path);
        // SAFETY: as above, restoring the previous mask.
        unsafe { libc::umask(old) };
        let listener = bound.map_err(|e| format!("Could not listen on {shown}: {e}"))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("Could not set the mode of {shown}: {e}"))?;
        Ok(listener)
    }

    /// Whether the peer runs as this process's effective user (S6).
    pub fn peer_is_same_user(stream: &UnixStream) -> io::Result<bool> {
        // SAFETY: geteuid has no preconditions.
        let me = unsafe { libc::geteuid() };
        Ok(peer_uid(stream)? == me)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn peer_uid(stream: &UnixStream) -> io::Result<libc::uid_t> {
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: cred and len are valid for writes of the sizes passed; the fd is open
        // for the duration of the call.
        let rc = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(cred.uid)
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn peer_uid(stream: &UnixStream) -> io::Result<libc::uid_t> {
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        // SAFETY: uid and gid are valid for writes; the fd is open for the call.
        let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(uid)
    }
}
