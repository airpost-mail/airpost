import {
  KeyManager as MainKeyManager,
  registerKeyManagerIPC,
  secureStorage,
} from '../../src/main/profile/key-manager';
import { KEY_MANAGER_CHANNEL } from '../../src/engine/channels';
import { mailspringKeychain } from '../../src/main/profile/mailspring-keychain';

const CREDENTIALS_KEY = 'credentials';

// A stored blob is a JSON-serialized Buffer, which is what config.get() hands back.
const storedBlob = () => Buffer.from('v10encrypted').toJSON();

// The main process's KeyManager, on a plain-object config; nothing here reaches safeStorage, a
// keychain or a real dialog.
let KeyManager: MainKeyManager = null;

describe('KeyManager in the main process', function () {
  beforeEach(function () {
    this.config = {};
    KeyManager = new MainKeyManager({
      config: {
        get: (key: string) => this.config[key],
        set: (key: string, val: any) => {
          this.config[key] = val;
        },
      },
    });
    spyOn(secureStorage, 'isAvailable').andCallFake(() => Promise.resolve(true));
    spyOn(secureStorage, 'encrypt').andCallFake((plaintext: string) =>
      Promise.resolve(Buffer.from(plaintext))
    );
    // The sync API only matters for pre-1.24 blobs; specs that exercise it override this.
    spyOn(secureStorage, 'decryptLegacy').andCallFake(() => {
      throw new Error('sync backend unavailable');
    });
    // Reading saved passwords never reaches for Mailspring's keychain item.
    spyOn(mailspringKeychain, 'run').andCallFake(() =>
      Promise.reject(new Error('no keychain in specs'))
    );
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
      expect(mailspringKeychain.run).not.toHaveBeenCalled();
    });

    // Passwords copied from Mailspring are re-encrypted by Import from Mailspring; a blob in
    // config.json that Tern can't open is never tried with Mailspring's key.
    it('is fatal, and leaves the blob alone, when Tern cannot read one Mailspring wrote', async function () {
      const untouched = Buffer.from('v10mailspring').toJSON();
      this.config[CREDENTIALS_KEY] = untouched;
      spyOn(secureStorage, 'decrypt').andCallFake(() =>
        Promise.reject(new Error('Error while decrypting the ciphertext'))
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
      expect(mailspringKeychain.run).not.toHaveBeenCalled();
      expect(this.config[CREDENTIALS_KEY]).toBe(untouched);
      expect(Object.keys(this.config)).toEqual([CREDENTIALS_KEY]);
    });

    // The regression this guards: an unreadable blob resolving to {} let the next
    // read-modify-write mutator persist it over every account's password.
    it('does not overwrite stored credentials when decryption fails', async function () {
      const untouched = storedBlob();
      this.config[CREDENTIALS_KEY] = untouched;
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
      spyOn(KeyManager, '_reportFatalError');
      expect(await KeyManager._getKeyHash()).toEqual({});
      expect(KeyManager._reportFatalError).not.toHaveBeenCalled();
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
      spyOn(KeyManager, '_showMessageBoxSync').andReturn(1);
      spyOn(KeyManager, '_quit');

      for (let i = 0; i < 3; i++) {
        try {
          KeyManager._reportFatalError(new Error('nope'));
        } catch (err) {
          // always rethrows so callers abort
        }
      }
      expect((KeyManager._showMessageBoxSync as jasmine.Spy).calls.length).toBe(1);
      expect((KeyManager._quit as jasmine.Spy).calls.length).toBe(1);
    });
  });

  describe('moving the account secrets to the engine', function () {
    beforeEach(function () {
      this.config[CREDENTIALS_KEY] = Buffer.from(
        JSON.stringify({
          'me@example.com-imap': 'pw',
          'me@example.com-refresh-token': 'rt',
          'integration-deepl-key': 'k',
        })
      ).toJSON();
      spyOn(secureStorage, 'decrypt').andCallFake((blob: Buffer) =>
        Promise.resolve({ result: blob.toString(), shouldReEncrypt: false })
      );
    });

    it("keeps the keyset as it was and drops only the accounts' secrets", async function () {
      expect(KeyManager.credentialsMovedToEngine()).toBe(false);
      expect(await KeyManager.hasAccountSecrets('me@example.com')).toBe(true);
      await KeyManager.moveAccountSecretsToEngine();

      expect(KeyManager.credentialsMovedToEngine()).toBe(true);
      expect(await KeyManager._getKeyHash()).toEqual({ 'integration-deepl-key': 'k' });
      expect(await KeyManager.hasAccountSecrets('me@example.com')).toBe(false);
      const before = await KeyManager.keysBeforeEngine();
      expect(before['me@example.com-imap']).toBe('pw');

      // A later move (after a downgrade added secrets again) keeps the first copy.
      await KeyManager.replacePassword('me@example.com-imap', 'newer');
      await KeyManager.moveAccountSecretsToEngine();
      expect((await KeyManager.keysBeforeEngine())['me@example.com-imap']).toBe('pw');
      expect(await KeyManager._getKeyHash()).toEqual({ 'integration-deepl-key': 'k' });
    });

    it('answers the windows from the kept keyset once the secrets moved', async function () {
      await KeyManager.moveAccountSecretsToEngine();
      expect(await KeyManager.accountSecrets('me@example.com')).toEqual({
        imap_password: undefined,
        smtp_password: undefined,
        refresh_token: undefined,
      });
      expect(await KeyManager.accountSecretsForWindows('me@example.com')).toEqual({
        imap_password: 'pw',
        smtp_password: undefined,
        refresh_token: 'rt',
      });
    });

    it('reads an unreadable kept keyset as empty', async function () {
      this.config.credentialsBeforeEngine = 'garbage';
      (secureStorage.decrypt as any).andCallFake(() => Promise.reject(new Error('no')));
      spyOn(console, 'warn');
      expect(await KeyManager.keysBeforeEngine()).toEqual({});
    });
  });

  // A profile another build made: safeStorage's key opens only for the build that wrote it.
  describe("with a keyset this build can't read", function () {
    beforeEach(function () {
      this.blob = storedBlob();
      this.config[CREDENTIALS_KEY] = this.blob;
      this.config.credentialsBeforeEngine = storedBlob();
      spyOn(secureStorage, 'decrypt').andCallFake(() =>
        Promise.reject(new Error('Error while decrypting the ciphertext'))
      );
      spyOn(KeyManager, '_reportFatalError').andCallFake((err: Error) => {
        throw err;
      });
      spyOn(console, 'error');
      spyOn(console, 'warn');
    });

    it('is fatal before the move, which it holds the only copy for', async function () {
      let raised: Error = null;
      try {
        await KeyManager.keysIfReadable();
      } catch (err) {
        raised = err;
      }
      expect(raised).not.toBe(null);
      expect(KeyManager._reportFatalError).toHaveBeenCalled();
    });

    describe('after the move', function () {
      beforeEach(function () {
        this.config.credentialsMovedToEngine = true;
      });

      it('reads as none, with one warning', async function () {
        expect(await KeyManager.keysIfReadable()).toBe(null);
        expect(await KeyManager.getPassword('integration-deepl-key')).toBe(undefined);
        expect(await KeyManager.accountSecretsForWindows('me@example.com')).toEqual({
          imap_password: undefined,
          smtp_password: undefined,
          refresh_token: undefined,
        });
        expect(KeyManager._reportFatalError).not.toHaveBeenCalled();
        expect((console.warn as jasmine.Spy).calls.length).toBe(2);
      });

      it('skips deletes and refuses writes, leaving the blob as it is', async function () {
        await KeyManager.deleteAccountSecrets({ emailAddress: 'me@example.com' });
        await KeyManager.deletePassword('integration-deepl-key');
        let raised: Error = null;
        try {
          await KeyManager.replacePassword('integration-deepl-key', 'k');
        } catch (err) {
          raised = err;
        }
        expect(raised).not.toBe(null);
        expect(KeyManager._reportFatalError).not.toHaveBeenCalled();
        expect(secureStorage.encrypt).not.toHaveBeenCalled();
        expect(this.config[CREDENTIALS_KEY]).toBe(this.blob);
      });
    });
  });

  // A new profile, where the engine takes every secret: safeStorage is never reached.
  describe('with no keyset', function () {
    beforeEach(function () {
      spyOn(secureStorage, 'decrypt');
    });

    it('moves, stores an account without secrets and deletes without safeStorage', async function () {
      await KeyManager.moveAccountSecretsToEngine();
      expect(KeyManager.credentialsMovedToEngine()).toBe(true);
      await KeyManager.extractAndStoreAccountSecrets({
        emailAddress: 'me@example.com',
        settings: { imap_host: 'imap.example.com' },
      });
      await KeyManager.deleteAccountSecrets({ emailAddress: 'me@example.com' });
      expect(secureStorage.isAvailable).not.toHaveBeenCalled();
      expect(secureStorage.encrypt).not.toHaveBeenCalled();
      expect(secureStorage.decrypt).not.toHaveBeenCalled();
      expect(this.config[CREDENTIALS_KEY]).toBe(undefined);
    });
  });

  describe('for the windows', function () {
    beforeEach(function () {
      this.config[CREDENTIALS_KEY] = Buffer.from('{"a-imap":"secret"}').toJSON();
      spyOn(secureStorage, 'decrypt').andCallFake((blob: Buffer) =>
        Promise.resolve({ result: blob.toString(), shouldReEncrypt: false })
      );
      const handlers: { [channel: string]: (...args: any[]) => any } = {};
      registerKeyManagerIPC(
        { handle: (channel: string, fn: any) => (handlers[channel] = fn) } as any,
        KeyManager
      );
      this.call = (method: string, ...args: any[]) =>
        handlers[KEY_MANAGER_CHANNEL]({}, method, ...args);
    });

    it('answers the facade, and refuses what it does not offer', async function () {
      expect(await this.call('getPassword', 'a-imap')).toEqual({ value: 'secret' });
      expect(await this.call('_writeKeyHash', {})).toEqual({
        error: 'KeyManager has no method _writeKeyHash',
      });
    });

    it('stores account secrets without sending them back', async function () {
      const reply = await this.call('storeAccountSecrets', {
        emailAddress: 'me@example.com',
        settings: { imap_password: 'pw', refresh_token: 'rt' },
      });
      expect(reply).toEqual({ value: undefined });
      expect(await this.call('accountSecrets', 'me@example.com')).toEqual({
        value: { imap_password: 'pw', smtp_password: undefined, refresh_token: 'rt' },
      });
    });

    // Windows and the engine's token rotations all write the one keyset.
    it('keeps every change when several arrive at once', async function () {
      await Promise.all([
        this.call('replacePassword', 'b-imap', 'one'),
        this.call('replacePassword', 'c-imap', 'two'),
        this.call('deletePassword', 'a-imap'),
      ]);
      expect(JSON.parse(this.config[CREDENTIALS_KEY].toString())).toEqual({
        'b-imap': 'one',
        'c-imap': 'two',
      });
    });
  });
});
