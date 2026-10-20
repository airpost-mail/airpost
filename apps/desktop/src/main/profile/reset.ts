// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The file half of "Reset Accounts and Settings" (application:reset-profile in application.ts,
which first destroys the windows and stops the engine), also used by Import from Mailspring
before it restarts to copy a Mailspring profile in. Everything in the profile goes except
Chromium's single-instance lock, which the running app still holds, and the profile marker.
*/
import fs from 'fs';
import os from 'os';
import path from 'path';
import { localized } from '../../intl/localize';
import { TERN_PROFILE_MARKER, hasTernProfileMarker } from './marker';

// The running instance's single-instance lock: SingletonLock, SingletonSocket and SingletonCookie
// on macOS and Linux, `lockfile` on Windows (held open, so it can't be deleted there anyway).
// The profile marker stays so the emptied directory can be reset again.
function isKept(name: string) {
  return name.startsWith('Singleton') || name === 'lockfile' || name === TERN_PROFILE_MARKER;
}

// Chromium's storage for the default session, which clearStorageData has already emptied. On
// Windows the network service and the storage backends keep some of these open, so they can
// fail to delete; that is logged, not reported to the user.
const CHROMIUM_STORAGE = new Set([
  'Cookies',
  'Cookies-journal',
  'Network',
  'Local Storage',
  'Session Storage',
  'IndexedDB',
  'Service Worker',
  'Shared Dictionary',
  'SharedStorage',
  'WebStorage',
  'blob_storage',
  'DIPS',
  'Trust Tokens',
  'GPUCache',
  'Code Cache',
  'DawnGraphiteCache',
  'DawnWebGPUCache',
]);

// Case-insensitive on macOS and Windows (their default filesystems), symlinks resolved, so
// `/users/me` or a link to the home folder compares equal to it.
function canonicalPath(p: string) {
  let resolved = path.resolve(p);
  try {
    resolved = fs.realpathSync.native(resolved);
  } catch {
    // A path that doesn't exist yet compares as given.
  }
  return process.platform === 'darwin' || process.platform === 'win32'
    ? resolved.toLowerCase()
    : resolved;
}

function isSameOrAncestor(dir: string, of: string) {
  if (dir === of) return true;
  return of.startsWith(dir.endsWith(path.sep) ? dir : dir + path.sep);
}

// Only empties a directory Tern marked as its profile (marker.ts), and never the
// filesystem root, the home folder, the folders that hold every app's profile, or a folder that
// contains one of them.
export function canResetProfileDirectory(configDirPath: string, protectedPaths: string[] = []) {
  if (!configDirPath || !path.isAbsolute(configDirPath)) return false;
  const dir = canonicalPath(configDirPath);
  if (dir === canonicalPath(path.parse(dir).root)) return false;
  const protectedDirs = [os.homedir(), ...protectedPaths].filter(Boolean).map(canonicalPath);
  if (protectedDirs.some((p) => isSameOrAncestor(dir, p))) return false;
  return hasTernProfileMarker(configDirPath);
}

/**
 * Deletes the contents of `configDirPath`. Returns the entries that could not be deleted (eg. a
 * file another process holds open on Windows), leaving out Chromium storage that is already
 * empty.
 */
export function resetProfileDirectory(configDirPath: string, protectedPaths: string[] = []) {
  if (!canResetProfileDirectory(configDirPath, protectedPaths)) {
    throw new Error(`Refusing to delete the contents of ${configDirPath}`);
  }
  const failed: string[] = [];
  for (const name of fs.readdirSync(configDirPath)) {
    if (isKept(name)) continue;
    const entry = path.join(configDirPath, name);
    try {
      fs.rmSync(entry, { recursive: true, force: true, maxRetries: 3 });
    } catch (err) {
      console.error(`Could not delete ${entry}: ${err}`);
      if (!CHROMIUM_STORAGE.has(name)) failed.push(entry);
    }
  }
  return failed;
}

export function profileResetDetail(configDirPath: string) {
  return localized(
    `Everything in %@ is deleted: your accounts and their saved passwords, settings, signatures and templates, the local mail cache, downloaded attachments, installed plugins and themes, and logs. Mail on your servers is not affected.\n\nTern then restarts with no accounts.`,
    configDirPath
  );
}
