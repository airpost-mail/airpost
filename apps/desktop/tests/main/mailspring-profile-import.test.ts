import fs from 'fs';
import os from 'os';
import path from 'path';
import Sqlite3 from 'better-sqlite3';
import {
  SKIP_MAILSPRING_IMPORT_MARKER,
  importMailspringProfile,
  shouldImportMailspringProfile,
} from '../../src/main/profile/mailspring-import';

function write(file: string, contents = 'x') {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, contents);
}

// Every file under `dir` with its contents and mtime, to prove the source is left untouched.
function snapshot(dir: string) {
  const entries: Record<string, string> = {};
  const walk = (current: string) => {
    for (const name of fs.readdirSync(current)) {
      const full = path.join(current, name);
      const stat = fs.lstatSync(full);
      const rel = path.relative(dir, full);
      if (stat.isDirectory()) {
        entries[rel] = `dir ${stat.mtimeMs}`;
        walk(full);
      } else {
        entries[rel] = `${stat.mtimeMs} ${fs.readFileSync(full).toString('base64')}`;
      }
    }
  };
  walk(dir);
  return entries;
}

describe('importMailspringProfile', function () {
  let root: string;
  let from: string;
  let to: string;
  let db: Sqlite3.Database;

  beforeEach(function () {
    spyOn(console, 'log');
    spyOn(console, 'error');
    root = fs.mkdtempSync(path.join(os.tmpdir(), 'profile-import-spec-'));
    from = path.join(root, 'Mailspring');
    to = path.join(root, 'Tern');

    write(path.join(from, 'config.json'), '{"*":{"accounts":[]}}');
    write(path.join(from, 'packages', 'theme', 'package.json'), '{}');
    write(path.join(from, 'files', 'ab', 'cd', 'attachment.pdf'), 'pdf');
    write(path.join(from, 'disable-hardware-acceleration'), 'marker');
    write(path.join(from, 'Local Storage', 'leveldb', 'CURRENT'), 'ls');
    write(path.join(from, 'mailsync-abc.log'), 'log');
    write(path.join(from, 'mailsync-abc.log.1'), 'log');
    write(path.join(from, 'Cache', 'data_0'), 'cache');
    write(path.join(from, 'Code Cache', 'js', 'index'), 'cache');
    write(path.join(from, 'GPUCache', 'data_0'), 'cache');
    write(path.join(from, 'DawnGraphiteCache', 'data_0'), 'cache');
    write(path.join(from, 'Crashpad', 'settings.dat'), 'crash');
    write(path.join(from, 'compile-cache', 'ts', 'a.js'), 'cache');
    write(path.join(from, 'SingletonCookie'), 'lock');
    write(path.join(from, 'Local Storage', 'Cache', 'nested'), 'cache');

    // Left open in WAL mode with an uncheckpointed write, as a running Mailspring.app keeps it.
    db = new Sqlite3(path.join(from, 'edgehill.db'));
    db.pragma('journal_mode = WAL');
    db.pragma('wal_autocheckpoint = 0');
    db.exec('CREATE TABLE Thread (id TEXT PRIMARY KEY, subject TEXT)');
    db.prepare('INSERT INTO Thread VALUES (?, ?)').run('t1', 'in the WAL');
  });

  afterEach(function () {
    if (db.open) db.close();
    fs.rmSync(root, { recursive: true, force: true });
  });

  it('copies the profile, skipping caches, crash dumps, locks and logs', async function () {
    expect(await importMailspringProfile(from, to)).toBe(true);

    for (const rel of [
      'config.json',
      'packages/theme/package.json',
      'files/ab/cd/attachment.pdf',
      'disable-hardware-acceleration',
      'Local Storage/leveldb/CURRENT',
    ]) {
      expect(fs.readFileSync(path.join(to, rel), 'utf8')).toEqual(
        fs.readFileSync(path.join(from, rel), 'utf8')
      );
    }
    for (const rel of [
      'mailsync-abc.log',
      'mailsync-abc.log.1',
      'Cache',
      'Code Cache',
      'GPUCache',
      'DawnGraphiteCache',
      'Crashpad',
      'compile-cache',
      'SingletonCookie',
      'Local Storage/Cache',
      'edgehill.db',
      'edgehill.db-wal',
      'tern.db-wal',
    ]) {
      expect(fs.existsSync(path.join(to, rel))).toBe(false);
    }

    const copy = new Sqlite3(path.join(to, 'tern.db'), { readonly: true });
    expect(copy.prepare('SELECT subject FROM Thread WHERE id = ?').get('t1')).toEqual({
      subject: 'in the WAL',
    });
    copy.close();
  });

  it('leaves the source profile unchanged', async function () {
    const before = snapshot(from);
    await importMailspringProfile(from, to);
    // The -shm index is shared memory every reader of a WAL database updates.
    const after = snapshot(from);
    delete before['edgehill.db-shm'];
    delete after['edgehill.db-shm'];
    expect(after).toEqual(before);
  });

  it('copies the files other than the database before returning', function () {
    const pending = importMailspringProfile(from, to);
    expect(fs.existsSync(path.join(to, 'disable-hardware-acceleration'))).toBe(true);
    expect(fs.existsSync(path.join(to, 'config.json'))).toBe(false);
    return pending;
  });

  it('does nothing once the Tern profile has a config.json', async function () {
    write(path.join(to, 'config.json'), '{"tern":true}');
    expect(shouldImportMailspringProfile(from, to)).toBe(false);
    expect(await importMailspringProfile(from, to)).toBe(false);
    expect(fs.readFileSync(path.join(to, 'config.json'), 'utf8')).toEqual('{"tern":true}');
    expect(fs.existsSync(path.join(to, 'packages'))).toBe(false);
  });

  it('does nothing after a reset left the skip marker', async function () {
    write(path.join(to, SKIP_MAILSPRING_IMPORT_MARKER));
    expect(shouldImportMailspringProfile(from, to)).toBe(false);
    expect(await importMailspringProfile(from, to)).toBe(false);
    expect(fs.readdirSync(to)).toEqual([SKIP_MAILSPRING_IMPORT_MARKER]);
  });

  it('does nothing without a Mailspring profile', async function () {
    const missing = path.join(root, 'Mailspring-dev');
    expect(await importMailspringProfile(missing, to)).toBe(false);
    expect(fs.existsSync(to)).toBe(false);
  });

  it('copies a profile that has no database', async function () {
    db.close();
    for (const name of fs.readdirSync(from).filter((n) => n.startsWith('edgehill.db'))) {
      fs.rmSync(path.join(from, name));
    }
    expect(await importMailspringProfile(from, to)).toBe(true);
    expect(fs.existsSync(path.join(to, 'config.json'))).toBe(true);
    expect(fs.existsSync(path.join(to, 'tern.db'))).toBe(false);
  });

  it('removes what it copied when the database copy fails, so the next launch retries', async function () {
    // Something already in the Tern profile (eg. the single-instance lock) stays.
    write(path.join(to, 'SingletonLock'), 'ours');
    fs.mkdirSync(path.join(to, 'tern.db'));

    let error = null;
    try {
      await importMailspringProfile(from, to);
    } catch (err) {
      error = err;
    }
    expect(error).not.toBe(null);
    expect(fs.readdirSync(to)).toEqual(['SingletonLock']);
    expect(shouldImportMailspringProfile(from, to)).toBe(true);
  });
});
