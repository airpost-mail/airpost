// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import fs from 'fs';
import path from 'path';
import { atomicWriteFileSync } from '../../utils/fs';

/*
The files half of Import from Mailspring (Preferences > Accounts, onboarding); the dialogs and the
keychain step are in mailspring-import-flow.ts. The Mailspring profile is only ever read:
Mailspring.app may be running on it.

What the import does depends on Tern:
- With no accounts, the whole profile comes over. The action empties the Tern profile the way
  Reset Accounts and Settings does, writes IMPORT_REQUEST_FILE and restarts;
  `--import-from-mailspring=<dir>` asks for the same on a profile that has no config.json yet.
  The next launch copies the profile in before Electron is ready (importMailspringProfile), so
  Chromium's storage, where Mailspring keeps its mail rules, is in place before Chromium opens
  it.
- With accounts, only the Mailspring accounts Tern doesn't have are added, with the passwords
  Mailspring saved (newMailspringAccounts), while Tern runs. Nothing else changes.

The copy skips caches, crash dumps, single-instance locks, engine logs, the compile cache and
Chromium's Local State (Tern keeps its own key; the keychain step reads Mailspring's). edgehill.db
is copied with SQLite's online backup, since Mailspring.app may have it open in WAL mode, to
tern.db, where the engine's migrate moves the C++ database aside and import-legacy carries local
drafts and metadata over. config.json is written last, with the calendar settings under Tern's
keys and the saved passwords as the keychain step re-encrypted them.
*/

const CONFIG_FILE = 'config.json';
const LOCAL_STATE_FILE = 'Local State';
const MAILSPRING_DATABASE_FILE = 'edgehill.db';
const DATABASE_FILE = 'tern.db';
const DATABASE_SUFFIXES = ['', '-wal', '-shm'];

// Written into the emptied Tern profile by the action; the next launch imports from the profile
// it names and then removes it.
export const IMPORT_REQUEST_FILE = 'import-from-mailspring.json';

// Settings Mailspring keeps under keys Tern names differently.
const RENAMED_SETTINGS: Array<[string, string]> = [
  ['mailspring.disabledCalendars', 'core.calendar.disabledCalendars'],
  ['mailspring.calendarView', 'core.calendar.view'],
];

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
  return (
    name !== CONFIG_FILE &&
    name !== LOCAL_STATE_FILE &&
    !name.startsWith(MAILSPRING_DATABASE_FILE) &&
    !isSkipped(name)
  );
}

export function hasMailspringProfile(dir: string | null): dir is string {
  if (!dir) return false;
  try {
    return fs.statSync(path.join(dir, CONFIG_FILE)).isFile();
  } catch (err) {
    return false;
  }
}

export interface MailspringSettings {
  [key: string]: any;
}

export interface MailspringAccountJSON {
  id: string;
  emailAddress: string;
  settings?: { [key: string]: any };
  [key: string]: any;
}

// The settings in the profile's config.json. Throws when there are none.
export function readMailspringSettings(dir: string): MailspringSettings {
  const json = JSON.parse(fs.readFileSync(path.join(dir, CONFIG_FILE)).toString());
  const settings = json && json['*'];
  if (!settings || typeof settings !== 'object') {
    throw new Error(`${path.join(dir, CONFIG_FILE)} holds no settings`);
  }
  return settings;
}

// The accounts in Mailspring's settings that have an id and an address, without duplicates.
export function mailspringAccounts(settings: MailspringSettings): MailspringAccountJSON[] {
  const seen = new Set<string>();
  const result: MailspringAccountJSON[] = [];
  for (const json of Array.isArray(settings.accounts) ? settings.accounts : []) {
    if (!json || typeof json.id !== 'string' || typeof json.emailAddress !== 'string') continue;
    const email = json.emailAddress.toLowerCase();
    if (seen.has(json.id) || seen.has(email)) continue;
    seen.add(json.id);
    seen.add(email);
    result.push(json);
  }
  return result;
}

// The Mailspring accounts Tern doesn't have: none with the same id or address (in any case).
export function newMailspringAccounts(
  mailspring: MailspringAccountJSON[],
  tern: Array<{ id?: string; emailAddress?: string }>
) {
  const ids = new Set(tern.map((a) => a && a.id).filter(Boolean));
  const emails = new Set(
    tern.map((a) => a && typeof a.emailAddress === 'string' && a.emailAddress.toLowerCase())
  );
  return mailspring.filter((a) => !ids.has(a.id) && !emails.has(a.emailAddress.toLowerCase()));
}

export function requestMailspringImport(to: string, from: string) {
  fs.writeFileSync(path.join(to, IMPORT_REQUEST_FILE), JSON.stringify({ from }));
}

function clearMailspringImportRequest(to: string) {
  try {
    fs.rmSync(path.join(to, IMPORT_REQUEST_FILE), { force: true });
  } catch (err) {
    console.error(`Could not remove ${IMPORT_REQUEST_FILE}: ${err}`);
  }
}

/**
 * The Mailspring profile this launch imports: the one the last session asked for, or
 * `fromFlag` (--import-from-mailspring) when the Tern profile has no config.json yet. Null
 * when there is none.
 */
export function pendingMailspringImport(to: string, fromFlag?: string): string | null {
  const requestPath = path.join(to, IMPORT_REQUEST_FILE);
  if (fs.existsSync(requestPath)) {
    try {
      const request = JSON.parse(fs.readFileSync(requestPath).toString());
      if (request && typeof request.from === 'string' && request.from) return request.from;
    } catch (err) {
      console.error(`${IMPORT_REQUEST_FILE} is unreadable: ${err}`);
    }
    clearMailspringImportRequest(to);
    return null;
  }
  if (fromFlag && !fs.existsSync(path.join(to, CONFIG_FILE))) return path.resolve(fromFlag);
  return null;
}

function valueAt(settings: MailspringSettings, keyPath: string) {
  let node: any = settings;
  for (const key of keyPath.split('.')) {
    if (!node || typeof node !== 'object') return undefined;
    node = node[key];
  }
  return node;
}

function setValueAt(settings: MailspringSettings, keyPath: string, value: unknown) {
  const keys = keyPath.split('.');
  let node: any = settings;
  for (const key of keys.slice(0, -1)) {
    if (!node[key] || typeof node[key] !== 'object') node[key] = {};
    node = node[key];
  }
  node[keys[keys.length - 1]] = value;
}

// Deletes the value, then each parent object it leaves empty.
function deleteValueAt(settings: MailspringSettings, keyPath: string) {
  const keys = keyPath.split('.');
  const parents: any[] = [settings];
  for (const key of keys.slice(0, -1)) {
    const next = parents[parents.length - 1][key];
    if (!next || typeof next !== 'object') return;
    parents.push(next);
  }
  delete parents[parents.length - 1][keys[keys.length - 1]];
  for (let i = parents.length - 1; i > 0; i--) {
    if (Object.keys(parents[i]).length) break;
    delete parents[i - 1][keys[i - 1]];
  }
}

/**
 * Mailspring's settings as Tern's config.json holds them: the renamed keys moved, and
 * `credentials` replaced by the keyset the keychain step re-encrypted (none when it is null).
 */
export function importedSettings(settings: MailspringSettings, credentials: unknown) {
  const next: MailspringSettings = JSON.parse(JSON.stringify(settings));
  for (const [from, to] of RENAMED_SETTINGS) {
    const value = valueAt(next, from);
    if (value === undefined) continue;
    if (valueAt(next, to) === undefined) setValueAt(next, to, value);
    deleteValueAt(next, from);
  }
  if (credentials) next.credentials = credentials;
  else delete next.credentials;
  return next;
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

export interface ImportSteps {
  // The keychain step, once the app is ready, given Mailspring's stored `credentials`: the
  // keyset to store as Tern's `credentials`, re-encrypted with Tern's key, or null to import
  // without passwords. Rejects to cancel the import.
  credentials(stored: unknown): Promise<unknown>;
}

export interface ImportResult {
  accounts: number;
  // Mailspring saved passwords that did not come over; the accounts need signing in again.
  passwordsLeftBehind: boolean;
}

// A failure the user chose (cancelling at the keychain step), which needs no error dialog.
export class ImportCancelledError extends Error {
  constructor() {
    super('The import from Mailspring was cancelled');
    this.name = 'ImportCancelledError';
  }
}

/**
 * Copies the Mailspring profile `from` into the Tern profile `to`. Every file except the
 * database and config.json is copied before this returns, so files read before Electron's
 * `ready` event (eg. the software-rendering marker) are already in place. On failure
 * everything copied so far is removed again and the promise rejects; the request is removed
 * either way.
 */
export function importMailspringProfile(
  from: string,
  to: string,
  steps: ImportSteps
): Promise<ImportResult> {
  console.log(`Importing the Mailspring profile from ${from} into ${to}`);
  const created: string[] = [];
  const fail = (err: unknown) => {
    console.error(`Importing the Mailspring profile failed: ${err}`);
    removeEntries(to, created);
    clearMailspringImportRequest(to);
    return Promise.reject(err);
  };

  let settings: MailspringSettings;
  try {
    settings = readMailspringSettings(from);
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

  const database = path.join(from, MAILSPRING_DATABASE_FILE);
  created.push(...DATABASE_SUFFIXES.map((suffix) => `${DATABASE_FILE}${suffix}`));
  const copyDatabase = fs.existsSync(database)
    ? backupDatabase(database, path.join(to, DATABASE_FILE))
    : Promise.resolve();

  return copyDatabase
    .then(() => steps.credentials(settings.credentials))
    .then((credentials) => {
      created.push(CONFIG_FILE);
      const config = { '*': importedSettings(settings, credentials) };
      atomicWriteFileSync(path.join(to, CONFIG_FILE), JSON.stringify(config, null, 2));
      clearMailspringImportRequest(to);
      console.log('Imported the Mailspring profile');
      const stored = settings.credentials;
      return {
        accounts: mailspringAccounts(settings).length,
        passwordsLeftBehind: !credentials && !!stored && stored !== 'null',
      };
    })
    .catch(fail);
}
