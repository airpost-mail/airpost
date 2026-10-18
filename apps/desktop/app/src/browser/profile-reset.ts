// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The file half of "Reset Accounts and Settings" (application:reset-profile in application.ts,
which first destroys the windows and stops the engine). Everything in the profile goes except
Chromium's single-instance lock, which the running app still holds, and the marker that stops
the next launch from importing the Mailspring profile again is written last, into the emptied
directory.
*/
import fs from 'fs';
import os from 'os';
import path from 'path';
import { localized } from '../intl';
import { SKIP_MAILSPRING_IMPORT_MARKER } from './mailspring-profile-import';

// SingletonLock, SingletonSocket and SingletonCookie belong to this running instance.
function isKept(name: string) {
  return name.startsWith('Singleton');
}

// Refuses a directory whose contents aren't only Tern's: the filesystem root, the home folder
// and the folders that hold every app's profile (a --config-dir-path pointing at them).
export function canResetProfileDirectory(configDirPath: string, protectedPaths: string[] = []) {
  if (!configDirPath || !path.isAbsolute(configDirPath)) return false;
  const dir = path.resolve(configDirPath);
  if (dir === path.parse(dir).root) return false;
  return ![os.homedir(), ...protectedPaths].filter(Boolean).some((p) => path.resolve(p) === dir);
}

/**
 * Deletes the contents of `configDirPath`, then writes the import marker. Returns the entries
 * that could not be deleted (eg. a file another process holds open on Windows); the marker is
 * written either way.
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
      failed.push(entry);
    }
  }
  fs.writeFileSync(
    path.join(configDirPath, SKIP_MAILSPRING_IMPORT_MARKER),
    'Written by Reset Accounts and Settings. While this file exists, Tern does not import a Mailspring profile.\n'
  );
  return failed;
}

export function profileResetDetail(configDirPath: string) {
  return localized(
    `Everything in %@ is deleted: your accounts and their saved passwords, settings, signatures and templates, the local mail cache, downloaded attachments, installed plugins and themes, and logs. Mail on your servers is not affected.\n\nTern then restarts with no accounts, and doesn't import a Mailspring profile again.`,
    configDirPath
  );
}
