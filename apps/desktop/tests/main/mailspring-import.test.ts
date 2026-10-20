import crypto from 'crypto';
import fs from 'fs';
import os from 'os';
import path from 'path';
import Sqlite3 from 'better-sqlite3';
import {
  hasMailspringProfile,
  IMPORT_REQUEST_FILE,
  ImportCancelledError,
  importedSettings,
  importMailspringProfile,
  ImportSteps,
  newMailspringAccounts,
  pendingMailspringImport,
  requestMailspringImport,
} from '../../src/main/profile/mailspring-import';
import {
  addMailspringAccounts,
  importDialogs,
  mailspringCredentialsForTern,
} from '../../src/main/profile/mailspring-import-flow';
import { KeyManager, secureStorage } from '../../src/main/profile/key-manager';
import { mailspringKeychain } from '../../src/main/profile/mailspring-keychain';

// A synthetic stand-in for the "Mailspring Safe Storage" password; never a real keychain item.
const SECRET = 'c3ludGhldGljLW1haWxzcHJpbmc=';

// Encrypts the way Chromium's macOS KeychainKeyProvider does.
const mailspringBlob = (plaintext: string) => {
  const key = crypto.pbkdf2Sync(SECRET, 'saltysalt', 1003, 16, 'sha1');
  const cipher = crypto.createCipheriv('aes-128-cbc', key, Buffer.alloc(16, ' '));
  return Buffer.concat([Buffer.from('v10'), cipher.update(plaintext, 'utf8'), cipher.final()]);
};

const MAILSPRING_KEYS = {
  'a@example.com-imap': 'imap-a',
  'a@example.com-smtp': 'smtp-a',
  'b@example.com-refresh-token': 'token-b',
};

const account = (id: string, emailAddress: string) => ({
  __cls: 'Account',
  id,
  emailAddress,
  provider: 'imap',
  settings: { imap_host: 'imap.example.com' },
});

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
        entries[rel] = `${stat.mtimeMs} ${stat.mode} ${fs.readFileSync(full).toString('base64')}`;
      }
    }
  };
  walk(dir);
  // The -shm index is shared memory every reader of a WAL database updates.
  delete entries['edgehill.db-shm'];
  return entries;
}

const rejection = async (promise: Promise<any>) => {
  try {
    await promise;
  } catch (err) {
    return err;
  }
  return null;
};

describe('Import from Mailspring', function () {
  let root: string;
  let from: string;
  let to: string;
  let db: Sqlite3.Database;

  beforeEach(function () {
    spyOn(console, 'log');
    spyOn(console, 'warn');
    spyOn(console, 'error');
    root = fs.mkdtempSync(path.join(os.tmpdir(), 'mailspring-import-spec-'));
    from = path.join(root, 'Mailspring');
    to = path.join(root, 'Tern');

    const settings = {
      accounts: [account('acc-a', 'a@example.com'), account('acc-b', 'b@example.com')],
      accountsVersion: 4,
      credentials: mailspringBlob(JSON.stringify(MAILSPRING_KEYS)).toJSON(),
      core: { theme: 'ui-dark' },
      mailspring: { disabledCalendars: ['cal-off'], calendarView: 'Month' },
      signatures: { s1: { body: 'Best' } },
    };
    write(path.join(from, 'config.json'), JSON.stringify({ '*': settings }));
    write(path.join(from, 'packages', 'theme', 'package.json'), '{}');
    write(path.join(from, 'templates', 'Hello.html'), '<p>Hi</p>');
    write(path.join(from, 'files', 'ab', 'cd', 'attachment.pdf'), 'pdf');
    write(path.join(from, 'disable-hardware-acceleration'), 'marker');
    write(path.join(from, 'Local Storage', 'leveldb', 'CURRENT'), 'ls');
    write(path.join(from, 'Local State'), '{"os_crypt":{}}');
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

    // The Tern profile as the action leaves it: emptied, with the request in it.
    write(path.join(to, '.tern-profile'), 'marker');
    write(path.join(to, 'SingletonLock'), 'ours');
    requestMailspringImport(to, from);

    // The keychain step, stubbed: what it returns stands for Mailspring's keyset re-encrypted.
    this.ternCredentials = Buffer.from('tern-encrypted').toJSON();
    this.steps = {
      credentials: jasmine
        .createSpy('credentials')
        .andCallFake(() => Promise.resolve(this.ternCredentials)),
    } as ImportSteps;
  });

  afterEach(function () {
    if (db.open) db.close();
    fs.rmSync(root, { recursive: true, force: true });
  });

  const ternConfig = () => JSON.parse(fs.readFileSync(path.join(to, 'config.json'), 'utf8'))['*'];

  describe('finding a Mailspring profile', function () {
    it('is there when the profile has a config.json', function () {
      expect(hasMailspringProfile(from)).toBe(true);
      expect(hasMailspringProfile(path.join(root, 'nowhere'))).toBe(false);
      expect(hasMailspringProfile(null)).toBe(false);
      fs.mkdirSync(path.join(root, 'odd', 'config.json'), { recursive: true });
      expect(hasMailspringProfile(path.join(root, 'odd'))).toBe(false);
    });

    it('imports at launch only what the last session or the flag asked for', function () {
      expect(pendingMailspringImport(to)).toBe(from);
      fs.rmSync(path.join(to, IMPORT_REQUEST_FILE));
      expect(pendingMailspringImport(to)).toBe(null);
      // --import-from-mailspring only names a profile for a Tern profile with no config.json.
      expect(pendingMailspringImport(to, from)).toBe(from);
      write(path.join(to, 'config.json'), '{"*":{}}');
      expect(pendingMailspringImport(to, from)).toBe(null);
    });

    it('drops an unreadable request', function () {
      write(path.join(to, IMPORT_REQUEST_FILE), 'not json');
      expect(pendingMailspringImport(to)).toBe(null);
      expect(fs.existsSync(path.join(to, IMPORT_REQUEST_FILE))).toBe(false);
    });
  });

  describe('into a Tern profile with no accounts', function () {
    it('copies the profile, skipping caches, crash dumps, locks, logs and Local State', async function () {
      expect(await importMailspringProfile(from, to, this.steps)).toEqual({
        accounts: 2,
        passwordsLeftBehind: false,
      });

      for (const rel of [
        'packages/theme/package.json',
        'templates/Hello.html',
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
        'Local State',
        'Local Storage/Cache',
        'edgehill.db',
        'edgehill.db-wal',
        IMPORT_REQUEST_FILE,
      ]) {
        expect(fs.existsSync(path.join(to, rel))).toBe(false);
      }
      expect(fs.readFileSync(path.join(to, 'SingletonLock'), 'utf8')).toBe('ours');
    });

    it("copies Mailspring's edgehill.db to tern.db, writes in its WAL included", async function () {
      await importMailspringProfile(from, to, this.steps);
      const copy = new Sqlite3(path.join(to, 'tern.db'), { readonly: true });
      expect(copy.prepare('SELECT subject FROM Thread WHERE id = ?').get('t1')).toEqual({
        subject: 'in the WAL',
      });
      copy.close();
    });

    it("writes Mailspring's settings, under Tern's keys, with the re-encrypted passwords", async function () {
      await importMailspringProfile(from, to, this.steps);
      expect(this.steps.credentials).toHaveBeenCalledWith(
        mailspringBlob(JSON.stringify(MAILSPRING_KEYS)).toJSON()
      );
      const settings = ternConfig();
      expect(settings.accounts.map((a) => a.id)).toEqual(['acc-a', 'acc-b']);
      expect(settings.credentials).toEqual(this.ternCredentials);
      expect(settings.core.theme).toBe('ui-dark');
      expect(settings.core.calendar).toEqual({ disabledCalendars: ['cal-off'], view: 'Month' });
      expect(settings.mailspring).toBeUndefined();
      expect(settings.signatures).toEqual({ s1: { body: 'Best' } });
    });

    it('never changes the Mailspring profile', async function () {
      const before = snapshot(from);
      await importMailspringProfile(from, to, this.steps);
      expect(snapshot(from)).toEqual(before);
    });

    it('copies the files other than the database before returning', function () {
      const pending = importMailspringProfile(from, to, this.steps);
      expect(fs.existsSync(path.join(to, 'disable-hardware-acceleration'))).toBe(true);
      expect(fs.existsSync(path.join(to, 'config.json'))).toBe(false);
      return pending;
    });

    it('imports without passwords when the keychain step gives none', async function () {
      this.ternCredentials = null;
      expect(await importMailspringProfile(from, to, this.steps)).toEqual({
        accounts: 2,
        passwordsLeftBehind: true,
      });
      expect(ternConfig().credentials).toBeUndefined();
      expect(ternConfig().accounts.length).toBe(2);
    });

    it('removes what it copied when the keychain step is cancelled', async function () {
      const before = snapshot(from);
      (this.steps.credentials as jasmine.Spy).andCallFake(() =>
        Promise.reject(new ImportCancelledError())
      );
      const err = await rejection(importMailspringProfile(from, to, this.steps));
      expect(err instanceof ImportCancelledError).toBe(true);
      expect(fs.readdirSync(to).sort()).toEqual(['.tern-profile', 'SingletonLock']);
      expect(snapshot(from)).toEqual(before);
    });

    it('removes what it copied when the database copy fails', async function () {
      fs.mkdirSync(path.join(to, 'tern.db'));
      expect(await rejection(importMailspringProfile(from, to, this.steps))).not.toBe(null);
      expect(fs.readdirSync(to).sort()).toEqual(['.tern-profile', 'SingletonLock']);
      expect(this.steps.credentials).not.toHaveBeenCalled();
    });

    it('copies a profile that has no database', async function () {
      db.close();
      for (const name of fs.readdirSync(from).filter((n) => n.startsWith('edgehill.db'))) {
        fs.rmSync(path.join(from, name));
      }
      await importMailspringProfile(from, to, this.steps);
      expect(fs.existsSync(path.join(to, 'config.json'))).toBe(true);
      expect(fs.existsSync(path.join(to, 'tern.db'))).toBe(false);
    });

    it('copies nothing from a profile whose config.json is unreadable', async function () {
      write(path.join(from, 'config.json'), '{"no settings": true}');
      expect(await rejection(importMailspringProfile(from, to, this.steps))).not.toBe(null);
      expect(fs.readdirSync(to).sort()).toEqual(['.tern-profile', 'SingletonLock']);
    });
  });

  describe('the settings', function () {
    it("moves the calendar settings to Tern's keys and keeps the rest", function () {
      const next = importedSettings(
        { mailspring: { calendarView: 'Day', other: 1 }, core: { reading: { a: 1 } } },
        null
      );
      expect(next).toEqual({
        mailspring: { other: 1 },
        core: { reading: { a: 1 }, calendar: { view: 'Day' } },
      });
    });
  });

  describe('the keychain step', function () {
    beforeEach(function () {
      this.platform = mailspringKeychain.platform;
      mailspringKeychain.platform = 'darwin';
      spyOn(mailspringKeychain, 'run').andCallFake(() => Promise.resolve(Buffer.from(SECRET)));
      spyOn(secureStorage, 'isAvailable').andCallFake(() => Promise.resolve(true));
      spyOn(secureStorage, 'encrypt').andCallFake((plaintext: string) =>
        Promise.resolve(Buffer.from(`tern:${plaintext}`))
      );
      spyOn(importDialogs, 'showMessageBoxSync').andReturn(0);
    });

    afterEach(function () {
      mailspringKeychain.platform = this.platform;
    });

    const stored = () => mailspringBlob(JSON.stringify(MAILSPRING_KEYS)).toJSON();

    it("reads Mailspring's keychain item and re-encrypts with Tern's key", async function () {
      const credentials: any = await mailspringCredentialsForTern(stored(), from);
      expect(mailspringKeychain.run).toHaveBeenCalledWith('/usr/bin/security', [
        'find-generic-password',
        '-w',
        '-s',
        'Mailspring Safe Storage',
      ]);
      expect(credentials.toString()).toBe(`tern:${JSON.stringify(MAILSPRING_KEYS)}`);
    });

    it('goes on without the passwords when the user says so', async function () {
      (mailspringKeychain.run as jasmine.Spy).andCallFake(() =>
        Promise.reject(new Error('declined'))
      );
      (importDialogs.showMessageBoxSync as jasmine.Spy).andReturn(1);
      expect(await mailspringCredentialsForTern(stored(), from)).toBe(null);
    });

    it('cancels the import otherwise', async function () {
      (mailspringKeychain.run as jasmine.Spy).andCallFake(() =>
        Promise.reject(new Error('declined'))
      );
      const err = await rejection(mailspringCredentialsForTern(stored(), from));
      expect(err instanceof ImportCancelledError).toBe(true);
    });

    describe('with accounts in Tern', function () {
      beforeEach(function () {
        this.config = {
          accounts: [account('acc-t', 'tern@example.com'), account('other-id', 'B@Example.com')],
          accountsVersion: 7,
        };
        this.configObject = {
          get: (key: string) => this.config[key],
          set: (key: string, value: any) => {
            this.config[key] = value;
          },
        };
        this.keyManager = new KeyManager({ config: this.configObject });
        spyOn(secureStorage, 'decrypt').andCallFake((blob: Buffer) =>
          Promise.resolve({ result: blob.toString().replace(/^tern:/, ''), shouldReEncrypt: false })
        );
        this.add = () =>
          addMailspringAccounts({ from, config: this.configObject, keyManager: this.keyManager });
      });

      it("adds only the accounts Tern doesn't have, with their passwords", async function () {
        (importDialogs.showMessageBoxSync as jasmine.Spy).andReturn(1);
        const before = snapshot(from);
        expect(await this.add()).toBe('added');

        // b@example.com is already in Tern, under another id and case.
        expect(this.config.accounts.map((a) => a.id)).toEqual(['acc-t', 'other-id', 'acc-a']);
        expect(this.config.accounts[2].settings).toEqual({ imap_host: 'imap.example.com' });
        expect(this.config.accountsVersion).toBe(8);
        expect(await this.keyManager.accountSecrets('a@example.com')).toEqual({
          imap_password: 'imap-a',
          smtp_password: 'smtp-a',
          refresh_token: undefined,
        });
        expect(await this.keyManager.hasAccountSecrets('b@example.com')).toBe(false);
        expect(snapshot(from)).toEqual(before);
        expect(fs.existsSync(path.join(to, 'config.json'))).toBe(false);
      });

      it('changes nothing when the user cancels', async function () {
        (importDialogs.showMessageBoxSync as jasmine.Spy).andReturn(0);
        expect(await this.add()).toBe('cancelled');
        expect(this.config.accounts.length).toBe(2);
        expect(this.config.accountsVersion).toBe(7);
        expect(mailspringKeychain.run).not.toHaveBeenCalled();
      });

      it('says so when Tern has every Mailspring account', async function () {
        this.config.accounts.push(account('acc-a', 'a@example.com'));
        expect(await this.add()).toBe('none');
        expect(importDialogs.showMessageBoxSync).toHaveBeenCalled();
        expect(mailspringKeychain.run).not.toHaveBeenCalled();
        expect(this.config.accountsVersion).toBe(7);
      });
    });
  });

  it('matches accounts by id or by address in any case', function () {
    const tern = [{ id: 'x', emailAddress: 'A@EXAMPLE.COM' }, { id: 'acc-b' }];
    expect(
      newMailspringAccounts(
        [
          account('acc-a', 'a@example.com'),
          account('acc-b', 'b@example.com'),
          account('acc-c', 'c@example.com'),
        ],
        tern
      ).map((a) => a.id)
    ).toEqual(['acc-c']);
  });
});
