import fs from 'fs';
import path from 'path';

/**
 * Copies a Mailspring profile into an empty Tern profile on first launch.
 *
 * The Mailspring profile is only ever read: Mailspring.app may still be running on it. Caches,
 * crash dumps, single-instance locks, engine logs and the compile cache are skipped. The database
 * is copied with SQLite's online backup, since Mailspring.app may have it open in WAL mode, and
 * config.json is written last: its presence is what marks a Tern profile as set up, so a launch
 * that dies part-way retries next time. The copied C++ database is then handled by the normal
 * startup (MailsyncProcess.migrate moves it aside and import-legacy carries drafts and metadata
 * over).
 */

const CONFIG_FILE = 'config.json';
const DATABASE_FILE = 'edgehill.db';

// Written by "Reset Accounts and Settings" (src/main/profile/reset.ts) into the emptied
// profile, so the relaunch starts empty instead of copying the Mailspring profile in again.
export const SKIP_MAILSPRING_IMPORT_MARKER = 'skip-mailspring-import';

const SKIPPED_NAMES = new Set([
  'Cache',
  'Code Cache',
  'GPUCache',
  'GPUPersistentCache',
  'Crashpad',
  'Crash Reports',
  'compile-cache',
]);

function isSkipped(name: string) {
  return (
    SKIPPED_NAMES.has(name) ||
    /^Dawn.*Cache$/.test(name) ||
    name.startsWith('Singleton') ||
    /^mailsync-.*\.log/.test(name)
  );
}

// Top-level entries copied file by file. The database (with its -wal/-shm) and config.json are
// copied separately.
function isCopiedEntry(name: string) {
  return name !== CONFIG_FILE && !name.startsWith(DATABASE_FILE) && !isSkipped(name);
}

export function shouldImportMailspringProfile(from: string, to: string) {
  return (
    !fs.existsSync(path.join(to, CONFIG_FILE)) &&
    !fs.existsSync(path.join(to, SKIP_MAILSPRING_IMPORT_MARKER)) &&
    fs.existsSync(path.join(from, CONFIG_FILE))
  );
}

async function backupDatabase(source: string, destination: string) {
  const Database = require('better-sqlite3');
  const db = new Database(source, { readonly: true, fileMustExist: true });
  try {
    // Copy every page in one step: in smaller steps, each write Mailspring.app makes in between
    // restarts the backup from the first page.
    await db.backup(destination, { progress: () => 0x7fffffff });
  } finally {
    db.close();
  }
}

function removeEntries(dir: string, names: string[]) {
  for (const name of names) {
    try {
      fs.rmSync(path.join(dir, name), { recursive: true, force: true });
    } catch (err) {
      console.error(`Could not remove ${path.join(dir, name)} after a failed import: ${err}`);
    }
  }
}

/**
 * Copies `from` into `to` when `to` has no config.json and `from` has one. Resolves to whether
 * a profile was imported. On failure everything copied so far is removed again, leaving `to`
 * without a config.json, and the promise rejects.
 *
 * Every file except the database and config.json is copied before this returns, so files read
 * before Electron's `ready` event (eg. the software-rendering marker) are already in place.
 */
export function importMailspringProfile(from: string, to: string): Promise<boolean> {
  if (!shouldImportMailspringProfile(from, to)) {
    return Promise.resolve(false);
  }

  console.log(`Importing the Mailspring profile from ${from} into ${to}`);
  const created: string[] = [];
  const fail = (err: Error) => {
    console.error(`Importing the Mailspring profile failed, will retry next launch: ${err}`);
    removeEntries(to, created);
    return Promise.reject(err);
  };

  try {
    fs.mkdirSync(to, { recursive: true });
    for (const name of fs.readdirSync(from).filter(isCopiedEntry)) {
      created.push(name);
      fs.cpSync(path.join(from, name), path.join(to, name), {
        recursive: true,
        force: true,
        preserveTimestamps: true,
        verbatimSymlinks: true,
        filter: (source) => !isSkipped(path.basename(source)),
      });
    }
  } catch (err) {
    return fail(err);
  }

  const database = path.join(from, DATABASE_FILE);
  const copyDatabase = fs.existsSync(database)
    ? backupDatabase(database, path.join(to, DATABASE_FILE))
    : Promise.resolve();
  created.push(DATABASE_FILE, `${DATABASE_FILE}-wal`, `${DATABASE_FILE}-shm`);

  return copyDatabase
    .then(() => {
      created.push(CONFIG_FILE);
      fs.copyFileSync(path.join(from, CONFIG_FILE), path.join(to, CONFIG_FILE));
      console.log('Imported the Mailspring profile');
      return true;
    })
    .catch(fail);
}
