import fs from 'fs';
import path from 'path';
import { localized } from '../../intl/localize';
import { KEY_MANAGER_CHANNEL, KeyManagerMethod, KeyManagerReply } from '../../engine/channels';

export interface KeySet {
  [key: string]: string;
}

interface DecryptResult {
  result: string;
  shouldReEncrypt: boolean;
}

// The account JSON as config.json and the engine hold it; only these fields are read here.
export interface AccountSecretsJSON {
  id?: string;
  emailAddress: string;
  settings?: { [key: string]: any };
  [key: string]: any;
}

export interface AccountSecrets {
  imap_password?: string;
  smtp_password?: string;
  refresh_token?: string;
}

// The part of Config that KeyManager uses: the main process's config, a fake in specs.
export interface KeyManagerConfig {
  get(keyPath: string): any;
  set(keyPath: string, value: any): unknown;
}

const SECRET_SETTINGS = ['imap_password', 'smtp_password', 'refresh_token'];

export const configCredentialsKey = 'credentials';

// Set once the accounts' secrets moved into the engine (credentials.import, stage 11). The blob
// as it was then is kept under credentialsBeforeEngine for one release, so a downgrade and
// the next upgrade find the passwords; `credentials` keeps only what isn't an account's
// (integration keys, which the desktop still reads).
export const configCredentialsMovedKey = 'credentialsMovedToEngine';
export const configCredentialsBeforeEngineKey = 'credentialsBeforeEngine';

// The flag as config.json has it on disk, for launch before Config is loaded.
export function credentialsMovedToEngine(configDirPath: string) {
  try {
    const json = JSON.parse(fs.readFileSync(path.join(configDirPath, 'config.json')).toString());
    return !!(json && json['*'] && json['*'][configCredentialsMovedKey]);
  } catch (err) {
    return false;
  }
}

// `<email>-imap`, `<email>-smtp`, `<email>-refresh-token`.
const ACCOUNT_SECRET_KEY = /-(imap|smtp|refresh-token)$/;

export function isAccountSecretKey(name: string) {
  return ACCOUNT_SECRET_KEY.test(name);
}

// safeStorage is a main-process module; specs run in a renderer and replace every method here.
const safeStorage = () => require('electron').safeStorage;

// The real keychain varies by platform and may be locked or absent, so specs replace this.
export const secureStorage = {
  isAvailable: (): Promise<boolean> => safeStorage().isAsyncEncryptionAvailable(),
  encrypt: (plaintext: string): Promise<Buffer> => safeStorage().encryptStringAsync(plaintext),
  decrypt: (encrypted: Buffer): Promise<DecryptResult> =>
    safeStorage().decryptStringAsync(encrypted),
  // The synchronous API is kept only to read blobs written by 1.23 and earlier. On KWallet the
  // two APIs key off different wallet entries (sync: "Chromium Keys", async: "<app name> Keys"),
  // so a 1.23 blob is unreadable through decryptStringAsync even though both are tagged "v11".
  decryptLegacy: (encrypted: Buffer): string => safeStorage().decryptString(encrypted),
};

// On Linux, Chromium's async keyring always has a last-resort provider whose key is a hardcoded
// constant ("peanuts"); its ciphertexts are tagged "v10" instead of a real keyring's "v11".
// isAsyncEncryptionAvailable() resolves true even when only that provider initialized, so the
// tag is the only signal that the desktop keyring is unusable.
const usesPlaintextFallbackKey = (encrypted: Buffer) =>
  process.platform === 'linux' && encrypted.subarray(0, 3).toString() === 'v10';

/**
 * A basic wrap around electron's secure key management. Consolidates all of
 * our keys under a single namespaced keymap.
 *
 * Consolidating this prevents a ton of key authorization popups for each
 * and every key we want to access.
 *
 * It runs in the main process, which owns safeStorage and the `serve` engine's stdin; windows
 * reach it through the facade in src/window/key-manager.ts.
 */
export class KeyManager {
  private _fatalErrorReported = false;
  private _reEncryptAttempted = false;
  private _unreadableWarned = false;
  // Read-modify-write changes run one at a time: every window and the engine's token
  // rotations write the one keyset.
  private _writes: Promise<unknown> = Promise.resolve();
  config: KeyManagerConfig;

  constructor({ config }: { config: KeyManagerConfig }) {
    this.config = config;
  }

  async deleteAccountSecrets(account: { emailAddress: string }) {
    await this._delete((keys) => {
      delete keys[`${account.emailAddress}-imap`];
      delete keys[`${account.emailAddress}-smtp`];
      delete keys[`${account.emailAddress}-refresh-token`];
    });
  }

  // Stores the account's secrets and returns a copy of its JSON without them.
  async extractAndStoreAccountSecrets(account: AccountSecretsJSON) {
    const settings = account.settings || {};
    await this._change((keys) => {
      keys[`${account.emailAddress}-imap`] = settings.imap_password;
      keys[`${account.emailAddress}-smtp`] = settings.smtp_password;
      keys[`${account.emailAddress}-refresh-token`] = settings.refresh_token;
    });
    const next = JSON.parse(JSON.stringify(account));
    next.settings = next.settings || {};
    for (const key of SECRET_SETTINGS) delete next.settings[key];
    return next as AccountSecretsJSON;
  }

  async accountSecrets(emailAddress: string, keys: KeySet | null = null): Promise<AccountSecrets> {
    if (!keys) keys = await this._getKeyHash();
    return {
      imap_password: keys[`${emailAddress}-imap`],
      smtp_password: keys[`${emailAddress}-smtp`],
      refresh_token: keys[`${emailAddress}-refresh-token`],
    };
  }

  // A copy of the account's JSON with its secrets, as the engine takes it.
  async insertAccountSecrets(account: AccountSecretsJSON, keys: KeySet | null = null) {
    const next = JSON.parse(JSON.stringify(account));
    next.settings = {
      ...(next.settings || {}),
      ...(await this.accountSecrets(account.emailAddress, keys)),
    };
    return next as AccountSecretsJSON;
  }

  // Whether the keyset of this account has any secret.
  async hasAccountSecrets(emailAddress: string, keys: KeySet | null = null) {
    const secrets = await this.accountSecrets(emailAddress, keys);
    return Object.values(secrets).some((v) => typeof v === 'string' && v.length > 0);
  }

  // What windows read (OAuth revocation, the reconnect form): once the secrets moved to the
  // engine, a migrated account's come from the keyset kept from before the move. A keyset this
  // build can't read gives none, and the windows ask for a reconnect.
  async accountSecretsForWindows(emailAddress: string): Promise<AccountSecrets> {
    const secrets = await this.accountSecrets(emailAddress, (await this.keysIfReadable()) || {});
    if (!this.credentialsMovedToEngine() || Object.values(secrets).some(Boolean)) return secrets;
    return this.accountSecrets(emailAddress, await this.keysBeforeEngine());
  }

  credentialsMovedToEngine() {
    return !!this.config.get(configCredentialsMovedKey);
  }

  // The keyset as it was when it moved into the engine; {} when there is none or it can't be
  // read.
  async keysBeforeEngine(): Promise<KeySet> {
    const blob = this.config.get(configCredentialsBeforeEngineKey);
    if (!blob || blob === 'null') return {} as KeySet;
    try {
      const { result } = await secureStorage.decrypt(Buffer.from(blob, 'utf-8'));
      return JSON.parse(result) as KeySet;
    } catch (err) {
      console.warn('Airpost could not read the passwords kept from before the engine stored them.');
      return {} as KeySet;
    }
  }

  // After credentials.import took every account: keeps the blob as credentialsBeforeEngine
  // (the first time) and drops the accounts' secrets from `credentials`. A keyset with none
  // (a new profile) is left as it is, so the move never needs safeStorage then.
  moveAccountSecretsToEngine(): Promise<void> {
    const run = this._writes.then(async () => {
      const keys = await this._getKeyHash();
      if (!this.config.get(configCredentialsBeforeEngineKey)) {
        this.config.set(configCredentialsBeforeEngineKey, this.config.get(configCredentialsKey));
      }
      const accountKeys = Object.keys(keys).filter(isAccountSecretKey);
      if (accountKeys.length) {
        for (const name of accountKeys) delete keys[name];
        await this._writeKeyHash(keys);
      }
      this.config.set(configCredentialsMovedKey, true);
    });
    this._writes = run.catch(() => {});
    return run;
  }

  async replacePassword(keyName: string, newVal: string) {
    await this._change((keys) => {
      keys[keyName] = newVal;
    });
  }

  async deletePassword(keyName: string) {
    await this._delete((keys) => {
      delete keys[keyName];
    });
  }

  async getPassword(keyName: string) {
    try {
      const keys = await this.keysIfReadable();
      return keys ? keys[keyName] : undefined;
    } catch (err) {
      this._reportFatalError(err);
    }
  }

  /*
  Applies `mutate` to the keyset and stores it; a change that leaves it as it was writes nothing.
  Before the move a failure is fatal. After it the error goes to the caller only: the keyset
  then holds nothing the app needs to run (keysIfReadable).
  */
  _change(mutate: (keys: KeySet) => void): Promise<void> {
    const run = this._writes.then(async () => {
      try {
        const keys = await this._readKeyHash();
        const before = JSON.stringify(keys);
        mutate(keys);
        if (JSON.stringify(keys) !== before) await this._writeKeyHash(keys);
      } catch (err) {
        if (!this.credentialsMovedToEngine()) this._reportFatalError(err);
        throw err;
      }
    });
    this._writes = run.catch(() => {});
    return run;
  }

  // A delete after the move that can't be made is only warned about: what it would drop is a
  // copy the engine doesn't read.
  async _delete(mutate: (keys: KeySet) => void) {
    try {
      await this._change(mutate);
    } catch (err) {
      if (!this.credentialsMovedToEngine()) throw err;
      this._warnUnreadable(err);
    }
  }

  // The keyset; a stored one that can't be read is fatal: before the move it holds the only
  // copy of every account's secrets.
  async _getKeyHash(): Promise<KeySet> {
    try {
      return await this._readKeyHash();
    } catch (err) {
      this._reportFatalError(err);
    }
  }

  /*
  The keyset for the paths that can do without it once the engine holds the accounts' secrets
  (the move's leftovers, keys only an older engine reads): null, with a warning, when it can't be
  read then. safeStorage's key opens only for the build that made it (macOS ties its Keychain
  item to the code signature), so a keyset another build wrote must not stop this one. Before the
  move this is _getKeyHash.
  */
  async keysIfReadable(): Promise<KeySet | null> {
    if (!this.credentialsMovedToEngine()) return this._getKeyHash();
    try {
      return await this._readKeyHash();
    } catch (err) {
      this._warnUnreadable(err);
      return null;
    }
  }

  _warnUnreadable(err: unknown) {
    if (this._unreadableWarned) return;
    this._unreadableWarned = true;
    console.warn(
      `Airpost can't use the passwords it kept from before the engine stored them, and goes on without them: ${err}`
    );
  }

  async _readKeyHash(): Promise<KeySet> {
    const encryptedCredentials = this.config.get(configCredentialsKey);
    // Check for different null values to prevent issues if a migration from keytar has failed
    if (
      encryptedCredentials === undefined ||
      encryptedCredentials === null ||
      encryptedCredentials === 'null'
    ) {
      return {} as KeySet;
    }

    const encrypted = Buffer.from(encryptedCredentials, 'utf-8');
    let decrypted: DecryptResult;
    try {
      decrypted = await secureStorage.decrypt(encrypted);
    } catch (asyncErr) {
      try {
        // A blob from 1.23 or earlier. Flag it so it is rewritten with the async key below.
        decrypted = { result: secureStorage.decryptLegacy(encrypted), shouldReEncrypt: true };
      } catch (syncErr) {
        // Stored-but-unreadable is not the same as nothing stored. Resolving to an empty keyset
        // would let the next read-modify-write mutator persist it, erasing every account's
        // password over a locked keyring or a secret service that has not started yet.
        console.error('Airpost could not read saved passwords.', asyncErr, syncErr);
        throw new Error(
          localized('Airpost could not read your saved passwords and cannot continue.') +
            this._encryptionUnavailableHint()
        );
      }
    }

    let keys: KeySet;
    try {
      keys = JSON.parse(decrypted.result) as KeySet;
    } catch (err) {
      // Decrypted to something that is not a keyset: no passwords left to preserve.
      return {} as KeySet;
    }

    // Set by Chromium when the provider that encrypts new data is not the one this blob was
    // written with (e.g. Linux users moving to org.freedesktop.portal.Secret), and by the legacy
    // path above. The old key still decrypts, so this is hygiene: rewrite once, and never let a
    // failure become fatal.
    if (decrypted.shouldReEncrypt && !this._reEncryptAttempted) {
      this._reEncryptAttempted = true;
      try {
        await this._writeKeyHash(keys);
      } catch (err) {
        console.warn('Airpost could not re-encrypt your saved passwords with the new key.', err);
      }
    }

    return keys;
  }

  _encryptionUnavailableHint() {
    return process.platform === 'linux'
      ? localized(
          ' On Linux, Airpost stores passwords in your desktop keyring (KWallet, GNOME Keyring, or another Secret Service provider). Please make sure it is installed, running, and unlocked, then restart Airpost.'
        )
      : '';
  }

  async _writeKeyHash(keys: KeySet) {
    const unavailable = () =>
      new Error(
        localized(
          `Airpost could not store your password securely because encryption is not available on this system.`
        ) + this._encryptionUnavailableHint()
      );
    if (!(await secureStorage.isAvailable())) {
      throw unavailable();
    }
    const encryptedCredentials = await secureStorage.encrypt(JSON.stringify(keys));
    if (usesPlaintextFallbackKey(encryptedCredentials)) {
      throw unavailable();
    }
    this.config.set(configCredentialsKey, encryptedCredentials);
  }

  // Electron's main-process dialog and quit, which specs replace.
  _showMessageBoxSync(options: Electron.MessageBoxSyncOptions): number {
    return require('electron').dialog.showMessageBoxSync(options);
  }

  _quit() {
    require('electron').app.quit();
  }

  _reportFatalError(thrown: unknown): never {
    const err = thrown instanceof Error ? thrown : new Error(String(thrown));
    // ensureClients is throttled rather than serialized, and _getKeyHash's callers report its
    // throw a second time, so only the first failure gets a dialog. The rest still propagate.
    if (this._fatalErrorReported) {
      Object.assign(err, { noSentry: true });
      throw err;
    }
    this._fatalErrorReported = true;

    this._showMessageBoxSync({
      type: 'error',
      buttons: [localized('Quit')],
      message: err.message || localized(`Airpost could not store your password securely.`),
    });

    // tell the app to exit and rethrow the error to ensure code relying
    // on the passwords being saved never runs (saving identity for example).
    // Mark as user-visible so the global error handler does not also report
    // it to Sentry — the user has already been informed via the dialog above.
    Object.assign(err, { noSentry: true });
    this._quit();
    throw err;
  }
}

/*
Answers the facade in src/window/key-manager.ts. Errors go back as text, flagged when main already
showed a dialog for them.
*/
export function registerKeyManagerIPC(ipcMain: Electron.IpcMain, keyManager: KeyManager) {
  const methods: { [method in KeyManagerMethod]: (...args: any[]) => Promise<any> } = {
    // Windows never write the whole keyset, so an unreadable one can read as empty here.
    getKeyHash: async () => (await keyManager.keysIfReadable()) || {},
    accountSecrets: (emailAddress: string) => keyManager.accountSecretsForWindows(emailAddress),
    storeAccountSecrets: async (account: AccountSecretsJSON) => {
      await keyManager.extractAndStoreAccountSecrets(account);
    },
    deleteAccountSecrets: (emailAddress: string) =>
      keyManager.deleteAccountSecrets({ emailAddress }),
    getPassword: (keyName: string) => keyManager.getPassword(keyName),
    replacePassword: (keyName: string, value: string) => keyManager.replacePassword(keyName, value),
    deletePassword: (keyName: string) => keyManager.deletePassword(keyName),
  };
  ipcMain.handle(
    KEY_MANAGER_CHANNEL,
    async (_event, method: KeyManagerMethod, ...args: any[]): Promise<KeyManagerReply> => {
      const fn = Object.prototype.hasOwnProperty.call(methods, method) ? methods[method] : null;
      if (!fn) return { error: `KeyManager has no method ${method}` };
      try {
        return { value: await fn(...args) };
      } catch (err) {
        const fields = typeof err === 'object' && err !== null ? err : {};
        const message = 'message' in fields ? fields.message : undefined;
        const noSentry = 'noSentry' in fields ? fields.noSentry : undefined;
        return {
          error: (typeof message === 'string' && message) || `${err}`,
          noSentry: !!noSentry,
        };
      }
    }
  );
}
