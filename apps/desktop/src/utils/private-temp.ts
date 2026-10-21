// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Files written for another window or app to open: message bodies, raw source, logs, dragged
contacts. On Linux the temp dir is the shared /tmp, so each file gets its own fs.mkdtemp
directory (0700) and is written 0600: other users can't read it, and nobody can plant a file at
a name Tern will load. The directories this process made are removed when it goes away (a
window's renderer on unload).
*/
import fs from 'fs';
import os from 'os';
import path from 'path';

const created = new Set<string>();
let cleanupInstalled = false;

/** A new 0700 directory under the temp dir, removed when this process or window goes away. */
export function makePrivateTempDir(prefix = 'tern-'): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), prefix));
  created.add(dir);
  installCleanup();
  return dir;
}

/** Writes `name` at 0600 into a new private temp directory and returns its path. */
export function writePrivateTempFile(name: string, data: string | Buffer, prefix?: string) {
  const filePath = path.join(makePrivateTempDir(prefix), name);
  fs.writeFileSync(filePath, data, { mode: 0o600 });
  return filePath;
}

export function removePrivateTempDirs() {
  for (const dir of created) {
    try {
      fs.rmSync(dir, { recursive: true, force: true });
    } catch {
      // Best effort: the OS clears the temp dir eventually.
    }
  }
  created.clear();
}

function installCleanup() {
  if (cleanupInstalled) return;
  cleanupInstalled = true;
  if (typeof window !== 'undefined') window.addEventListener('unload', removePrivateTempDirs);
  process.once('exit', removePrivateTempDirs);
}
