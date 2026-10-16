import { localized } from './intl';
import { Account } from 'mailspring-exports';
import {
  canMigrate as canMigrateFromMailspring,
  decryptWithMailspringKey,
  MigrationError,
} from './mailspring-keychain-migration';

interface KeySet {
  [key: string]: string;
}

interface DecryptResult {
  result: string;
  shouldReEncrypt: boolean;
}

const { safeStorage } = require('@electron/remote');

const configCredentialsKey = 'credentials';

// "tern" once the stored credentials are known to be encrypted with Tern's own safeStorage key:
// Tern wrote them, read them, or migrated them. Until then an unreadable blob is taken to be one
// copied from Mailspring, and decrypted with Mailspring's key once.
const configCredentialsKeyOwnerKey = 'credentialsKeyOwner';
const TERN_OWNS_KEY = 'tern';

// The Mailspring blob, kept when it could not be migrated, so nothing copied over is destroyed.
const configUnmigratedCredentialsKey = 'credentialsFromMailspring';

// The real keychain varies by platform and may be locked or absent, so specs replace this.
export const secureStorage = {
  isAvailable: (): Promise<boolean> => safeStorage.isAsyncEncryptionAvailable(),
  encrypt: (plaintext: string): Promise<Buffer> => safeStorage.encryptStringAsync(plaintext),
  decrypt: (encrypted: Buffer): Promise<DecryptResult> => safeStorage.decryptStringAsync(encrypted),
  // The synchronous API is kept only to read blobs written by 1.23 and earlier. On KWallet the
  // two APIs key off different wallet entries (sync: "Chromium Keys", async: "<app name> Keys"),
  // so a 1.23 blob is unreadable through decryptStringAsync even though both are tagged "v11".
  decryptLegacy: (encrypted: Buffer): string => safeStorage.decryptString(encrypted),
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
 */
class KeyManager {
  private _fatalErrorReported = false;
  private _reEncryptAttempted = false;
  // Concurrent reads share one migration, so the user sees one keychain prompt.
  private _migration: Promise<KeySet | null> = null;

  async deleteAccountSecrets(account: Account) {
    try {
      const keys = await this._getKeyHash();
      delete keys[`${account.emailAddress}-imap`];
      delete keys[`${account.emailAddress}-smtp`];
      delete keys[`${account.emailAddress}-refresh-token`];
      await this._writeKeyHash(keys);
    } catch (err) {
      this._reportFatalError(err);
    }
  }

  async extractAndStoreAccountSecrets(account: Account) {
    try {
      const keys = await this._getKeyHash();
      keys[`${account.emailAddress}-imap`] = account.settings.imap_password;
      keys[`${account.emailAddress}-smtp`] = account.settings.smtp_password;
      keys[`${account.emailAddress}-refresh-token`] = account.settings.refresh_token;
      await this._writeKeyHash(keys);
    } catch (err) {
      this._reportFatalError(err);
    }
    const next = account.clone();
    delete next.settings.imap_password;
    delete next.settings.smtp_password;
    delete next.settings.refresh_token;
    return next;
  }

  async insertAccountSecrets(account: Account, keys: KeySet = null) {
    const next = account.clone();
    if (!keys) keys = await this._getKeyHash();
    next.settings.imap_password = keys[`${account.emailAddress}-imap`];
    next.settings.smtp_password = keys[`${account.emailAddress}-smtp`];
    next.settings.refresh_token = keys[`${account.emailAddress}-refresh-token`];
    return next;
  }

  async replacePassword(keyName: string, newVal: string) {
    try {
      const keys = await this._getKeyHash();
      keys[keyName] = newVal;
      await this._writeKeyHash(keys);
    } catch (err) {
      this._reportFatalError(err);
    }
  }

  async deletePassword(keyName: string) {
    try {
      const keys = await this._getKeyHash();
      delete keys[keyName];
      await this._writeKeyHash(keys);
    } catch (err) {
      this._reportFatalError(err);
    }
  }

  async getPassword(keyName: string) {
    try {
      const keys = await this._getKeyHash();
      return keys[keyName];
    } catch (err) {
      this._reportFatalError(err);
    }
  }

  async _getKeyHash(): Promise<KeySet> {
    const encryptedCredentials = AppEnv.config.get(configCredentialsKey);
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

    if (AppEnv.config.get(configCredentialsKeyOwnerKey) !== TERN_OWNS_KEY) {
      AppEnv.config.set(configCredentialsKeyOwnerKey, TERN_OWNS_KEY);
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
    if (AppEnv.config.get(configCredentialsKeyOwnerKey) === TERN_OWNS_KEY) {
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
    AppEnv.config.set(configUnmigratedCredentialsKey, AppEnv.config.get(configCredentialsKey));
    AppEnv.config.set(configCredentialsKey, null);
    AppEnv.config.set(configCredentialsKeyOwnerKey, TERN_OWNS_KEY);
  }

  // True when the user would rather sign in to each account again than retry next launch.
  _askToSignInAgainAfterMigrationFailed(): boolean {
    const response = require('@electron/remote').dialog.showMessageBoxSync({
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
    require('@electron/remote').app.quit();
    throw err;
  }

  _showMigrationFailedNotice() {
    require('@electron/remote').dialog.showMessageBox({
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
    AppEnv.config.set(configCredentialsKey, encryptedCredentials);
    AppEnv.config.set(configCredentialsKeyOwnerKey, TERN_OWNS_KEY);
  }

  _reportFatalError(err: Error): never {
    // ensureClients is throttled rather than serialized, and _getKeyHash's callers report its
    // throw a second time, so only the first failure gets a dialog. The rest still propagate.
    if (this._fatalErrorReported) {
      (err as any).noSentry = true;
      throw err;
    }
    this._fatalErrorReported = true;

    require('@electron/remote').dialog.showMessageBoxSync({
      type: 'error',
      buttons: [localized('Quit')],
      message: err.message || localized(`Tern could not store your password securely.`),
    });

    // tell the app to exit and rethrow the error to ensure code relying
    // on the passwords being saved never runs (saving identity for example).
    // Mark as user-visible so the global error handler does not also report
    // it to Sentry — the user has already been informed via the dialog above.
    (err as any).noSentry = true;
    require('@electron/remote').app.quit();
    throw err;
  }
}

export default new KeyManager();
