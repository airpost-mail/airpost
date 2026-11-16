// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The control socket: `pair` and `devices` tell the running gateway to re-read
//! `devices.json`, so a revoke closes that device's sessions at once.
//! One request line, `reload`, answered `ok`. Same-user peers only.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader as AsyncBufReader};
use tokio::net::UnixListener;

/// macOS limits socket paths to 104 bytes.
const MAX_SOCKET_PATH: usize = 100;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

/// `<state dir>/control.sock`, or, when that path is too long for a socket, a hash of it
/// under a per-user directory in the temp dir.
pub fn socket_path(state_dir: &Path) -> PathBuf {
    let p = state_dir.join("control.sock");
    if p.as_os_str().len() <= MAX_SOCKET_PATH {
        return p;
    }
    let digest = Sha256::digest(p.as_os_str().as_encoded_bytes());
    let name: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    std::env::temp_dir()
        .join(format!("airpost-gateway-{uid}"))
        .join(format!("{name}.sock"))
}

/// Binds the control socket (0600, in a 0700 directory), replacing a stale one. Fails when
/// another gateway answers on it.
pub fn bind(path: &Path) -> io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    if path.exists() {
        if StdUnixStream::connect(path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("another gateway is running ({})", path.display()),
            ));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Answers control requests until the listener fails. `reload` runs `on_reload`.
pub async fn serve(listener: UnixListener, on_reload: impl Fn() -> Result<(), String>) {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    loop {
        let stream = match listener.accept().await {
            Ok((s, _)) => s,
            Err(e) => {
                tracing::warn!(error = %e, "control socket accept failed");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        if !stream.peer_cred().is_ok_and(|c| c.uid() == uid) {
            tracing::warn!("refusing a control connection from another user");
            continue;
        }
        let (r, mut w) = stream.into_split();
        let mut line = String::new();
        let read = tokio::time::timeout(
            CLIENT_TIMEOUT,
            AsyncBufReader::new(r.take(64)).read_line(&mut line),
        )
        .await;
        let reply = match (read, line.trim()) {
            (Ok(Ok(_)), "reload") => match on_reload() {
                Ok(()) => "ok\n".to_owned(),
                Err(e) => format!("error {e}\n"),
            },
            _ => "error unknown request\n".to_owned(),
        };
        let _ = w.write_all(reply.as_bytes()).await;
    }
}

/// Asks a running gateway to reload. `Ok(false)` when none is running.
pub fn request_reload(path: &Path) -> io::Result<bool> {
    let mut stream = match StdUnixStream::connect(path) {
        Ok(s) => s,
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(false)
        }
        Err(e) => return Err(e),
    };
    stream.set_read_timeout(Some(CLIENT_TIMEOUT))?;
    stream.write_all(b"reload\n")?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    match reply.trim() {
        "ok" => Ok(true),
        other => Err(io::Error::other(format!("the gateway answered {other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn long_paths_move_to_the_temp_dir() {
        let short = Path::new("/tmp/t/gateway");
        assert_eq!(socket_path(short), short.join("control.sock"));
        let long = PathBuf::from(format!("/tmp/{}/gateway", "x".repeat(120)));
        let p = socket_path(&long);
        assert!(p.as_os_str().len() <= MAX_SOCKET_PATH + 40);
        assert_eq!(p, socket_path(&long));
    }

    #[tokio::test]
    async fn reload_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        assert!(!request_reload(&path).unwrap());
        let listener = bind(&path).unwrap();
        assert!(bind(&path).is_err(), "a second gateway must not take over");
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        tokio::spawn(serve(listener, move || {
            n2.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));
        let p = path.clone();
        assert!(tokio::task::spawn_blocking(move || request_reload(&p))
            .await
            .unwrap()
            .unwrap());
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }
}
