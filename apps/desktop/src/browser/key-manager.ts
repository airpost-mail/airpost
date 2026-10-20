import fs from 'fs';
import path from 'path';
import { localized } from '../intl';
import {
  canMigrate as canMigrateFromMailspring,
  decryptWithMailspringKey,
  MigrationError,
} from './mailspring-keychain-migration';
import { KEY_MANAGER_CHANNEL, KeyManagerMethod, KeyManagerReply } from '../supervisor-channels';

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

const configCredentialsKey = 'credentials';

// "tern" once the stored credentials are known to be encrypted with Tern's own safeStorage key:
// Tern wrote them, read them, or migrated them. Until then an unreadable blob is taken to be one
// copied from Mailspring, and decrypted with Mailspring's key once.
const configCredentialsKeyOwnerKey = 'credentialsKeyOwner';
const TERN_OWNS_KEY = 'tern';

// The Mailspring blob, kept when it could not be migrated, so nothing copied over is destroyed.
const configUnmigratedCredentialsKey = 'credentialsFromMailspring';

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
 * our keys under a single namespaced keymap and provides migration
 * support.
 *
 * Consolidating this prevents a ton of key authorization popups for each
 * and every key we want to access.
 *
 * It runs in the main process, which owns safeStorage and the `serve` engine's stdin; windows
 * reach it through the facade in src/key-manager.ts.
 */
export class KeyManager {
  private _fatalErrorReported = false;
  private _reEncryptAttempted = false;
  // Concurrent reads share one migration, so the user sees one keychain prompt.
  private _migration: Promise<KeySet | null> = null;
  // Read-modify-write changes run one at a time: every window and the engine's token
  // rotations write the one keyset.
  private _writes: Promise<unknown> = Promise.resolve();
  config: KeyManagerConfig;

  constructor({ config }: { config: KeyManagerConfig }) {
    this.config = config;
  }

  async deleteAccountSecrets(account: { emailAddress: string }) {
    await this._change((keys) => {
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

  async accountSecrets(emailAddress: string, keys: KeySet = null): Promise<AccountSecrets> {
    if (!keys) keys = await this._getKeyHash();
    return {
      imap_password: keys[`${emailAddress}-imap`],
      smtp_password: keys[`${emailAddress}-smtp`],
      refresh_token: keys[`${emailAddress}-refresh-token`],
    };
  }

  // A copy of the account's JSON with its secrets, as the engine takes it.
  async insertAccountSecrets(account: AccountSecretsJSON, keys: KeySet = null) {
    const next = JSON.parse(JSON.stringify(account));
    next.settings = {
      ...(next.settings || {}),
      ...(await this.accountSecrets(account.emailAddress, keys)),
    };
    return next as AccountSecretsJSON;
  }

  // Whether the keyset of this account has any secret.
  async hasAccountSecrets(emailAddress: string, keys: KeySet = null) {
    const secrets = await this.accountSecrets(emailAddress, keys);
    return Object.values(secrets).some((v) => typeof v === 'string' && v.length > 0);
  }

  // What windows read (OAuth revocation, the reconnect form): once the secrets moved to the
  // engine, a migrated account's come from the keyset kept from before the move.
  async accountSecretsForWindows(emailAddress: string): Promise<AccountSecrets> {
    const secrets = await this.accountSecrets(emailAddress);
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
      console.warn('Tern could not read the passwords kept from before the engine stored them.');
      return {} as KeySet;
    }
  }

  // After credentials.import took every account: keeps the blob as credentialsBeforeEngine
  // (the first time) and drops the accounts' secrets from `credentials`.
  moveAccountSecretsToEngine(): Promise<void> {
    const run = this._writes.then(async () => {
      const keys = await this._getKeyHash();
      if (!this.config.get(configCredentialsBeforeEngineKey)) {
        this.config.set(configCredentialsBeforeEngineKey, this.config.get(configCredentialsKey));
      }
      for (const name of Object.keys(keys)) {
        if (isAccountSecretKey(name)) delete keys[name];
      }
      await this._writeKeyHash(keys);
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
    await this._change((keys) => {
      delete keys[keyName];
    });
  }

  async getPassword(keyName: string) {
    try {
      const keys = await this._getKeyHash();
      return keys[keyName];
    } catch (err) {
      this._reportFatalError(err);
    }
  }

  _change(mutate: (keys: KeySet) => void): Promise<void> {
    const run = this._writes.then(async () => {
      try {
        const keys = await this._getKeyHash();
        mutate(keys);
        await this._writeKeyHash(keys);
      } catch (err) {
        this._reportFatalError(err);
      }
    });
    this._writes = run.catch(() => {});
    return run;
  }

  async _getKeyHash(): Promise<KeySet> {
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
        const migrated = await this._migrateFromMailspring(encrypted);
        if (migrated) return migrated;

        // Stored-but-unreadable is not the same as nothing stored. Resolving to an empty keyset
        // would let the next read-modify-write mutator persist it, erasing every account's
        // password over a locked keyring or a secret service that has not started yet.
        console.error('Tern could not read saved passwords.', asyncErr, syncErr);
        this._reportFatalError(
          new Error(
            localized('Tern could not read your saved passwords and cannot continue.') +
              this._encryptionUnavailableHint()
          )
        );
      }
    }

    let keys: KeySet;
    try {
      keys = JSON.parse(decrypted.result) as KeySet;
    } catch (err) {
      // A wrong key passes the padding check about once in 256 tries and yields garbage.
      const migrated = await this._migrateFromMailspring(encrypted);
      if (migrated) return migrated;
      // Decrypted to something that is not a keyset: no passwords left to preserve.
      return {} as KeySet;
    }

    if (this.config.get(configCredentialsKeyOwnerKey) !== TERN_OWNS_KEY) {
      this.config.set(configCredentialsKeyOwnerKey, TERN_OWNS_KEY);
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
        console.warn('Tern could not re-encrypt your saved passwords with the new key.', err);
      }
    }

    return keys;
  }

  // Resolves to the migrated keyset; to an empty one when the blob had to be set aside (the
  // accounts then ask for their passwords again); or to null when the failure is Tern's own
  // keyring, which is left to the fatal path above with nothing changed. Rejects, with nothing
  // changed, when the Mailspring secret was out of reach and the user chose to quit and retry.
  _migrateFromMailspring(encrypted: Buffer): Promise<KeySet | null> {
    if (this.config.get(configCredentialsKeyOwnerKey) === TERN_OWNS_KEY) {
      return Promise.resolve(null);
    }
    if (!canMigrateFromMailspring()) {
      return Promise.resolve(null);
    }
    if (!this._migration) {
      this._migration = this._runMigrationFromMailspring(encrypted);
    }
    return this._migration;
  }

  async _runMigrationFromMailspring(encrypted: Buffer): Promise<KeySet | null> {
    let keys: KeySet;
    try {
      const plaintext = await decryptWithMailspringKey(encrypted);
      keys = JSON.parse(plaintext) as KeySet;
      if (!keys || typeof keys !== 'object') throw new Error('not a keyset');
    } catch (err) {
      console.warn(`Tern could not bring over the passwords saved by Mailspring: ${err.message}`);
      if (err instanceof MigrationError && err.retryable) {
        // The secret may be readable next launch (keyring locked or not started yet, prompt
        // dismissed, lookup tool missing), so the copied blob is only set aside if the user says so.
        if (!this._askToSignInAgainAfterMigrationFailed()) {
          this._quitAfterDialog(err);
        }
        this._setAsideMailspringCredentials();
        return {} as KeySet;
      }
      // Missing, or not Mailspring's: give up for good rather than prompt every launch.
      this._setAsideMailspringCredentials();
      this._showMigrationFailedNotice();
      return {} as KeySet;
    }

    try {
      await this._writeKeyHash(keys);
    } catch (err) {
      // Tern's keyring is unavailable: keep the Mailspring blob and try again next launch.
      console.warn('Tern could not re-encrypt the passwords saved by Mailspring.', err);
      return null;
    }
    console.log('Moved the passwords saved by Mailspring to the Tern keychain item.');
    return keys;
  }

  // Mailspring's own profile still has its copy, and this one is kept aside.
  _setAsideMailspringCredentials() {
    this.config.set(configUnmigratedCredentialsKey, this.config.get(configCredentialsKey));
    this.config.set(configCredentialsKey, null);
    this.config.set(configCredentialsKeyOwnerKey, TERN_OWNS_KEY);
  }

  // True when the user would rather sign in to each account again than retry next launch.
  _askToSignInAgainAfterMigrationFailed(): boolean {
    const response = this._showMessageBoxSync({
      type: 'warning',
      buttons: [localized('Quit'), localized('Sign In Again')],
      defaultId: 0,
      cancelId: 0,
      message: localized('Tern could not read the passwords saved by Mailspring'),
      detail:
        localized(
          'Your accounts were copied from Mailspring, but the passwords Mailspring saved could not be read just now. Quit and open Tern again to retry, allowing access if your keychain asks, or sign in to each account again. Mailspring itself is not affected.'
        ) +
        (process.platform === 'linux'
          ? localized(
              ' On Linux, reading them needs your desktop keyring to be running and unlocked, and the secret-tool command (package libsecret-tools) or kwallet-query.'
            )
          : ''),
    });
    return response === 1;
  }

  // Ends the session like _reportFatalError, for a failure the user has already seen a dialog for.
  _quitAfterDialog(err: Error): never {
    this._fatalErrorReported = true;
    (err as any).noSentry = true;
    this._quit();
    throw err;
  }

  _showMigrationFailedNotice() {
    this._showMessageBox({
      type: 'warning',
      buttons: [localized('OK')],
      message: localized('Tern could not bring over your saved passwords'),
      detail: localized(
        'Your accounts were copied from Mailspring, but the passwords Mailspring saved could not be read. Tern will ask you to sign in to each account again. Mailspring itself is not affected.'
      ),
    });
  }

  _encryptionUnavailableHint() {
    return process.platform === 'linux'
      ? localized(
          ' On Linux, Tern stores passwords in your desktop keyring (KWallet, GNOME Keyring, or another Secret Service provider). Please make sure it is installed, running, and unlocked, then restart Tern.'
        )
      : '';
  }

  async _writeKeyHash(keys: KeySet) {
    const unavailable = () =>
      new Error(
        localized(
          `Tern could not store your password securely because encryption is not available on this system.`
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
    this.config.set(configCredentialsKeyOwnerKey, TERN_OWNS_KEY);
  }

  // Electron's main-process dialogs and quit, which specs replace.
  _showMessageBoxSync(options: Electron.MessageBoxSyncOptions): number {
    return require('electron').dialog.showMessageBoxSync(options);
  }

  _showMessageBox(options: Electron.MessageBoxOptions) {
    return require('electron').dialog.showMessageBox(options);
  }

  _quit() {
    require('electron').app.quit();
  }

  _reportFatalError(err: Error): never {
    // ensureClients is throttled rather than serialized, and _getKeyHash's callers report its
    // throw a second time, so only the first failure gets a dialog. The rest still propagate.
    if (this._fatalErrorReported) {
      (err as any).noSentry = true;
      throw err;
    }
    this._fatalErrorReported = true;

    this._showMessageBoxSync({
      type: 'error',
      buttons: [localized('Quit')],
      message: err.message || localized(`Tern could not store your password securely.`),
    });

    // tell the app to exit and rethrow the error to ensure code relying
    // on the passwords being saved never runs (saving identity for example).
    // Mark as user-visible so the global error handler does not also report
    // it to Sentry — the user has already been informed via the dialog above.
    (err as any).noSentry = true;
    this._quit();
    throw err;
  }
}

/*
Answers the facade in src/key-manager.ts. Errors go back as text, flagged when main already
showed a dialog for them.
*/
export function registerKeyManagerIPC(ipcMain: Electron.IpcMain, keyManager: KeyManager) {
  const methods: { [method in KeyManagerMethod]: (...args: any[]) => Promise<any> } = {
    getKeyHash: () => keyManager._getKeyHash(),
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
        return { error: (err && err.message) || `${err}`, noSentry: !!(err && err.noSentry) };
      }
    }
  );
}
