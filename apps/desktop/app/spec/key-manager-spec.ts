import crypto from 'crypto';
import KeyManager, { secureStorage } from '../src/key-manager';
import { mailspringKeychain, MigrationError } from '../src/mailspring-keychain-migration';

const CREDENTIALS_KEY = 'credentials';
const KEY_OWNER_KEY = 'credentialsKeyOwner';
const UNMIGRATED_KEY = 'credentialsFromMailspring';

// A synthetic stand-in for the "Mailspring Safe Storage" password; never a real keychain item.
const OLD_SECRET = 'c3ludGhldGljLW1haWxzcHJpbmc=';

// Encrypts the way Chromium's macOS KeychainKeyProvider does.
const mailspringBlob = (plaintext: string, secret = OLD_SECRET) => {
  const key = crypto.pbkdf2Sync(secret, 'saltysalt', 1003, 16, 'sha1');
  const cipher = crypto.createCipheriv('aes-128-cbc', key, Buffer.alloc(16, ' '));
  return Buffer.concat([Buffer.from('v10'), cipher.update(plaintext, 'utf8'), cipher.final()]);
};

// A stored blob is a JSON-serialized Buffer, which is what config.get() hands back.
const storedBlob = () => Buffer.from('v10encrypted').toJSON();

describe('KeyManager', function () {
  beforeEach(function () {
    // The manager is a singleton, so its once-per-session guards leak between specs.
    (KeyManager as any)._fatalErrorReported = false;
    (KeyManager as any)._reEncryptAttempted = false;
    (KeyManager as any)._migration = null;

    this.config = {};
    spyOn(AppEnv.config, 'get').andCallFake((key: string) => this.config[key]);
    spyOn(AppEnv.config, 'set').andCallFake((key: string, val: any) => {
      this.config[key] = val;
    });
    spyOn(secureStorage, 'isAvailable').andCallFake(() => Promise.resolve(true));
    spyOn(secureStorage, 'encrypt').andCallFake((plaintext: string) =>
      Promise.resolve(Buffer.from(plaintext))
    );
    // The sync API only matters for pre-1.24 blobs; specs that exercise it override this.
    spyOn(secureStorage, 'decryptLegacy').andCallFake(() => {
      throw new Error('sync backend unavailable');
    });
    // Specs must never reach the real keychain; ones that migrate replace this.
    this.platform = mailspringKeychain.platform;
    mailspringKeychain.platform = 'darwin';
    spyOn(mailspringKeychain, 'run').andCallFake(() =>
      Promise.reject(new Error('no keychain in specs'))
    );
    // The migration's dialogs are real Electron dialogs, and any spec whose blob fails to
    // decrypt reaches the migration.
    spyOn(KeyManager, '_showMigrationFailedNotice');
    spyOn(KeyManager, '_askToSignInAgainAfterMigrationFailed').andReturn(false);
    spyOn(KeyManager, '_quitAfterDialog').andCallFake((err: Error) => {
      throw err;
    });
  });

  afterEach(function () {
    mailspringKeychain.platform = this.platform;
  });

  describe('_getKeyHash', function () {
    it('returns an empty set when nothing has been stored', async function () {
      spyOn(secureStorage, 'decrypt');
      expect(await KeyManager._getKeyHash()).toEqual({});
      expect(secureStorage.decrypt).not.toHaveBeenCalled();
    });

    // A failed keytar migration can leave any of these behind.
    ['null', null, undefined].forEach((value) => {
      it(`returns an empty set for a ${JSON.stringify(value)} placeholder`, async function () {
        this.config[CREDENTIALS_KEY] = value;
        spyOn(secureStorage, 'decrypt');
        expect(await KeyManager._getKeyHash()).toEqual({});
        expect(secureStorage.decrypt).not.toHaveBeenCalled();
      });
    });

    it('returns the decrypted keys', async function () {
      this.config[CREDENTIALS_KEY] = storedBlob();
      spyOn(secureStorage, 'decrypt').andCallFake(() =>
        Promise.resolve({ result: '{"a-imap":"secret"}', shouldReEncrypt: false })
      );
      expect(await KeyManager._getKeyHash()).toEqual({ 'a-imap': 'secret' });
    });

    it('is fatal when a stored blob cannot be decrypted', async function () {
      this.config[CREDENTIALS_KEY] = storedBlob();
      this.config[KEY_OWNER_KEY] = 'tern';
      spyOn(secureStorage, 'decrypt').andCallFake(() =>
        Promise.reject(new Error('keyring locked'))
      );
      spyOn(KeyManager, '_reportFatalError').andCallFake((err: Error) => {
        throw err;
      });

      let raised: Error = null;
      try {
        await KeyManager._getKeyHash();
      } catch (err) {
        raised = err;
      }
      expect(raised).not.toBe(null);
      expect(KeyManager._reportFatalError).toHaveBeenCalled();
    });

    // The regression this guards: an unreadable blob resolving to {} let the next
    // read-modify-write mutator persist it over every account's password.
    it('does not overwrite stored credentials when decryption fails', async function () {
      const untouched = storedBlob();
      this.config[CREDENTIALS_KEY] = untouched;
      this.config[KEY_OWNER_KEY] = 'tern';
      spyOn(secureStorage, 'decrypt').andCallFake(() =>
        Promise.reject(new Error('keyring locked'))
      );
      spyOn(KeyManager, '_reportFatalError').andCallFake((err: Error) => {
        throw err;
      });

      try {
        await KeyManager.replacePassword('a-imap', 'new');
      } catch (err) {
        // replacePassword reports and rethrows.
      }
      expect(secureStorage.encrypt).not.toHaveBeenCalled();
      expect(this.config[CREDENTIALS_KEY]).toBe(untouched);
    });

    // 1.23 encrypted with the synchronous API, which on KWallet uses a different wallet entry
    // than the async provider, so those blobs only open through decryptString.
    it('falls back to the synchronous API for pre-1.24 blobs and re-encrypts them', async function () {
      this.config[CREDENTIALS_KEY] = storedBlob();
      spyOn(secureStorage, 'decrypt').andCallFake(() =>
        Promise.reject(new Error('Error while decrypting the ciphertext'))
      );
      (secureStorage.decryptLegacy as jasmine.Spy).andCallFake(() => '{"a-imap":"secret"}');
      spyOn(KeyManager, '_reportFatalError');

      expect(await KeyManager._getKeyHash()).toEqual({ 'a-imap': 'secret' });
      expect(KeyManager._reportFatalError).not.toHaveBeenCalled();
      expect(secureStorage.encrypt).toHaveBeenCalledWith('{"a-imap":"secret"}');
      expect(this.config[CREDENTIALS_KEY]).toEqual(Buffer.from('{"a-imap":"secret"}'));
    });

    it('starts over when the decrypted value is not a keyset', async function () {
      this.config[CREDENTIALS_KEY] = storedBlob();
      spyOn(secureStorage, 'decrypt').andCallFake(() =>
        Promise.resolve({ result: 'not json', shouldReEncrypt: false })
      );
      (mailspringKeychain.run as jasmine.Spy).andCallFake(() =>
        Promise.reject(new MigrationError('/usr/bin/security failed (44)', false))
      );
      spyOn(KeyManager, '_reportFatalError');
      expect(await KeyManager._getKeyHash()).toEqual({});
      expect(KeyManager._reportFatalError).not.toHaveBeenCalled();
    });
  });

  describe('migrating from Mailspring', function () {
    beforeEach(function () {
      spyOn(secureStorage, 'decrypt').andCallFake(() =>
        Promise.reject(new Error('Error while decrypting the ciphertext'))
      );
      spyOn(KeyManager, '_reportFatalError').andCallFake((err: Error) => {
        throw err;
      });
      this.config[CREDENTIALS_KEY] = mailspringBlob('{"a-imap":"secret"}').toJSON();
    });

    const givesSecret = (secret: string) =>
      (mailspringKeychain.run as jasmine.Spy).andCallFake(() =>
        Promise.resolve(Buffer.from(`${secret}\n`))
      );

    const readFails = async () => {
      let raised: Error = null;
      try {
        await KeyManager._getKeyHash();
      } catch (err) {
        raised = err;
      }
      return raised;
    };

    it('decrypts with the Mailspring key read-only and re-encrypts with the Tern key', async function () {
      givesSecret(OLD_SECRET);
      expect(await KeyManager._getKeyHash()).toEqual({ 'a-imap': 'secret' });

      const [file, args] = (mailspringKeychain.run as jasmine.Spy).calls[0].args;
      expect(file).toBe('/usr/bin/security');
      expect(args).toEqual(['find-generic-password', '-w', '-s', 'Mailspring Safe Storage']);
      expect(secureStorage.encrypt).toHaveBeenCalledWith('{"a-imap":"secret"}');
      expect(this.config[CREDENTIALS_KEY].toString()).toBe('{"a-imap":"secret"}');
      expect(this.config[KEY_OWNER_KEY]).toBe('tern');
      expect(KeyManager._showMigrationFailedNotice).not.toHaveBeenCalled();
    });

    it('prompts once for concurrent reads', async function () {
      givesSecret(OLD_SECRET);
      const results = await Promise.all([KeyManager._getKeyHash(), KeyManager._getKeyHash()]);
      expect(results).toEqual([{ 'a-imap': 'secret' }, { 'a-imap': 'secret' }]);
      expect((mailspringKeychain.run as jasmine.Spy).calls.length).toBe(1);
    });

    it('does not migrate credentials Tern wrote itself', async function () {
      givesSecret(OLD_SECRET);
      this.config[KEY_OWNER_KEY] = 'tern';
      expect(await readFails()).not.toBe(null);
      expect(mailspringKeychain.run).not.toHaveBeenCalled();
    });

    it('records the key owner after a normal read, so a later failure is not migrated', async function () {
      (secureStorage.decrypt as jasmine.Spy).andCallFake(() =>
        Promise.resolve({ result: '{"a-imap":"secret"}', shouldReEncrypt: false })
      );
      await KeyManager._getKeyHash();
      expect(this.config[KEY_OWNER_KEY]).toBe('tern');
      expect(mailspringKeychain.run).not.toHaveBeenCalled();
    });

    const lookupFails = (code: number | string) =>
      (mailspringKeychain.run as jasmine.Spy).andCallFake(() =>
        Promise.reject(new MigrationError(`/usr/bin/security failed (${code})`, code !== 44))
      );

    // Missing item, or a key that does not open the blob: the accounts ask for their passwords
    // again, the copied blob is kept aside, and the user is not asked again.
    const abandons = (name: string, setup: () => void) => {
      it(`sets the credentials aside once when ${name}`, async function () {
        setup();
        const original = this.config[CREDENTIALS_KEY];
        expect(await KeyManager._getKeyHash()).toEqual({});
        expect(KeyManager._reportFatalError).not.toHaveBeenCalled();
        expect((KeyManager._showMigrationFailedNotice as jasmine.Spy).calls.length).toBe(1);
        expect(this.config[CREDENTIALS_KEY]).toBe(null);
        expect(this.config[UNMIGRATED_KEY]).toBe(original);
        expect(this.config[KEY_OWNER_KEY]).toBe('tern');

        (KeyManager as any)._migration = null;
        expect(await KeyManager._getKeyHash()).toEqual({});
        expect((mailspringKeychain.run as jasmine.Spy).calls.length).toBeLessThan(2);
        expect((KeyManager._showMigrationFailedNotice as jasmine.Spy).calls.length).toBe(1);
      });
    };
    abandons('the keychain item does not exist', () => lookupFails(44));
    abandons('the secret is a different one', () => givesSecret('another-secret'));
    abandons('the lookup prints nothing', () => givesSecret(''));

    // Declined or missed prompt, timeout, locked or not-yet-started keyring, missing lookup tool:
    // the secret may be readable next launch, so nothing is set aside unless the user says so.
    describe('when the Mailspring secret is out of reach', function () {
      beforeEach(function () {
        this.signInAgain = false;
        (KeyManager._askToSignInAgainAfterMigrationFailed as jasmine.Spy).andCallFake(
          () => this.signInAgain
        );
      });

      const retries = (name: string, setup: (this: any) => void) => {
        it(`quits with nothing changed when ${name}, and asks again next launch`, async function () {
          setup.call(this);
          const original = this.config[CREDENTIALS_KEY];
          expect(await readFails()).not.toBe(null);
          expect(KeyManager._quitAfterDialog).toHaveBeenCalled();
          expect(KeyManager._showMigrationFailedNotice).not.toHaveBeenCalled();
          expect(this.config[CREDENTIALS_KEY]).toBe(original);
          expect(this.config[UNMIGRATED_KEY]).toBeUndefined();
          expect(this.config[KEY_OWNER_KEY]).toBeUndefined();

          const lookups = (mailspringKeychain.run as jasmine.Spy).calls.length;
          (KeyManager as any)._migration = null;
          expect(await readFails()).not.toBe(null);
          expect((mailspringKeychain.run as jasmine.Spy).calls.length).toBe(2 * lookups);
        });
      };
      retries('the keychain prompt is declined', () => lookupFails(51));
      retries('the lookup times out', () => lookupFails('null'));
      retries('the lookup fails unexpectedly', () => {});
      retries('the Linux keyring is not running yet', function () {
        mailspringKeychain.platform = 'linux';
        this.config[CREDENTIALS_KEY] = Buffer.from('v11encrypted').toJSON();
        (mailspringKeychain.run as jasmine.Spy).andCallFake((file: string) =>
          Promise.reject(new MigrationError(`${file} failed (1)`, true))
        );
      });

      it('sets the credentials aside when the user chooses to sign in again', async function () {
        lookupFails(51);
        this.signInAgain = true;
        const original = this.config[CREDENTIALS_KEY];
        expect(await KeyManager._getKeyHash()).toEqual({});
        expect(KeyManager._quitAfterDialog).not.toHaveBeenCalled();
        expect(KeyManager._showMigrationFailedNotice).not.toHaveBeenCalled();
        expect(this.config[CREDENTIALS_KEY]).toBe(null);
        expect(this.config[UNMIGRATED_KEY]).toBe(original);
        expect(this.config[KEY_OWNER_KEY]).toBe('tern');
      });
    });

    it('keeps the Mailspring blob and stays fatal when the Tern keyring is unavailable', async function () {
      givesSecret(OLD_SECRET);
      (secureStorage.isAvailable as jasmine.Spy).andCallFake(() => Promise.resolve(false));
      const original = this.config[CREDENTIALS_KEY];
      expect(await readFails()).not.toBe(null);
      expect(KeyManager._reportFatalError).toHaveBeenCalled();
      expect(this.config[CREDENTIALS_KEY]).toBe(original);
      expect(this.config[KEY_OWNER_KEY]).toBeUndefined();
      expect(KeyManager._showMigrationFailedNotice).not.toHaveBeenCalled();
    });

    // DPAPI is per user and its wrapped key is in the copied "Local State": nothing to migrate.
    it('does not migrate on Windows', async function () {
      mailspringKeychain.platform = 'win32';
      expect(await readFails()).not.toBe(null);
      expect(mailspringKeychain.run).not.toHaveBeenCalled();
      expect(this.config[KEY_OWNER_KEY]).toBeUndefined();
    });
  });

  describe('re-encryption', function () {
    beforeEach(function () {
      this.config[CREDENTIALS_KEY] = storedBlob();
      this.shouldReEncrypt = true;
      spyOn(secureStorage, 'decrypt').andCallFake(() =>
        Promise.resolve({ result: '{"a-imap":"secret"}', shouldReEncrypt: this.shouldReEncrypt })
      );
    });

    it('rewrites the blob when the key provider has changed', async function () {
      expect(await KeyManager._getKeyHash()).toEqual({ 'a-imap': 'secret' });
      expect(secureStorage.encrypt).toHaveBeenCalledWith('{"a-imap":"secret"}');
    });

    it('does not rewrite the blob when the provider is unchanged', async function () {
      this.shouldReEncrypt = false;
      await KeyManager._getKeyHash();
      expect(secureStorage.encrypt).not.toHaveBeenCalled();
    });

    it('only rewrites once per session', async function () {
      await KeyManager._getKeyHash();
      await KeyManager._getKeyHash();
      expect((secureStorage.encrypt as jasmine.Spy).calls.length).toBe(1);
    });

    // The keys just read are valid either way, so a failed rewrite must not quit the app.
    it('returns the keys and stays non-fatal when the rewrite fails', async function () {
      (secureStorage.isAvailable as jasmine.Spy).andCallFake(() => Promise.resolve(false));
      spyOn(KeyManager, '_reportFatalError');
      expect(await KeyManager._getKeyHash()).toEqual({ 'a-imap': 'secret' });
      expect(KeyManager._reportFatalError).not.toHaveBeenCalled();
    });
  });

  describe('_writeKeyHash', function () {
    it('throws when encryption is unavailable', async function () {
      (secureStorage.isAvailable as jasmine.Spy).andCallFake(() => Promise.resolve(false));
      let raised: Error = null;
      try {
        await KeyManager._writeKeyHash({ 'a-imap': 'secret' });
      } catch (err) {
        raised = err;
      }
      expect(raised).not.toBe(null);
      expect(secureStorage.encrypt).not.toHaveBeenCalled();
    });

    describe('on Linux', function () {
      beforeEach(function () {
        this.platform = Object.getOwnPropertyDescriptor(process, 'platform');
        Object.defineProperty(process, 'platform', { value: 'linux', configurable: true });
      });
      afterEach(function () {
        Object.defineProperty(process, 'platform', this.platform);
      });

      // isAsyncEncryptionAvailable() is true on Linux even when the keyring failed, because
      // Chromium falls back to a hardcoded key whose ciphertexts are tagged "v10".
      it('refuses to persist a blob encrypted with the hardcoded fallback key', async function () {
        (secureStorage.encrypt as jasmine.Spy).andCallFake(() =>
          Promise.resolve(Buffer.from('v10peanuts'))
        );
        let raised: Error = null;
        try {
          await KeyManager._writeKeyHash({ 'a-imap': 'secret' });
        } catch (err) {
          raised = err;
        }
        expect(raised).not.toBe(null);
        expect(this.config[CREDENTIALS_KEY]).toBeUndefined();
      });

      it('persists a blob encrypted with the desktop keyring', async function () {
        (secureStorage.encrypt as jasmine.Spy).andCallFake(() =>
          Promise.resolve(Buffer.from('v11keyring'))
        );
        await KeyManager._writeKeyHash({ 'a-imap': 'secret' });
        expect(this.config[CREDENTIALS_KEY]).toEqual(Buffer.from('v11keyring'));
      });
    });
  });

  describe('_reportFatalError', function () {
    it('shows one dialog no matter how many failures arrive', function () {
      const remote = require('@electron/remote');
      spyOn(remote.dialog, 'showMessageBoxSync').andReturn(1);
      spyOn(remote.app, 'quit');

      for (let i = 0; i < 3; i++) {
        try {
          KeyManager._reportFatalError(new Error('nope'));
        } catch (err) {
          // always rethrows so callers abort
        }
      }
      expect((remote.dialog.showMessageBoxSync as jasmine.Spy).calls.length).toBe(1);
      expect((remote.app.quit as jasmine.Spy).calls.length).toBe(1);
    });
  });
});
