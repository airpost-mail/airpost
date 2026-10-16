// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Owner-only files.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

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
    fn private_files_are_0600() {
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
