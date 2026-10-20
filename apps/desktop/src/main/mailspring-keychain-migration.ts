import crypto from 'crypto';
import { execFile } from 'child_process';

/**
 * Reads credentials that Mailspring encrypted, for profiles copied from Mailspring on first
 * launch (see main/mailspring-profile-import.ts).
 *
 * Electron's safeStorage keys its encryption off an OS secret named after the app. Tern's
 * secret is "Tern Safe Storage", so the copied keyset, encrypted under "Mailspring Safe Storage",
 * no longer opens through safeStorage. This module reads Mailspring's secret, read-only, and
 * decrypts the keyset itself, the way Chromium's os_crypt_async does:
 *
 * - macOS (KeychainKeyProvider): tag "v10", key = PBKDF2-HMAC-SHA1(keychain password,
 *   "saltysalt", 1003 iterations, 16 bytes).
 * - Linux (FreedesktopSecretKeyProvider): tag "v11", same derivation with 1 iteration, from the
 *   Secret Service item whose "application" attribute is the app name (KWallet: entry
 *   "<name> Safe Storage" in folder "<name> Keys"). A "v10" blob on Linux uses a constant key
 *   that does not depend on the app name, so Tern reads it without help.
 * - Both: AES-128-CBC, PKCS#7 padding, IV of 16 spaces (os_crypt_async/common/encryptor.cc).
 *
 * Windows needs none of this: DPAPI is per user, and the DPAPI-wrapped key lives in the
 * profile's "Local State", which the import copies.
 *
 * Mailspring's secret is only ever read, and is zeroed as soon as the key is derived.
 */

const OLD_APP_NAME = 'Mailspring';
const SALT = 'saltysalt';
const KEY_LENGTH = 16;
const IV = Buffer.alloc(16, ' ');
const KDF_ITERATIONS: { [platform: string]: number } = { darwin: 1003, linux: 1 };
const TAGS: { [platform: string]: string } = { darwin: 'v10', linux: 'v11' };

// Leaves time to answer the keychain prompt, but does not wait forever on a hung helper.
const LOOKUP_TIMEOUT_MS = 5 * 60 * 1000;

export type CommandRunner = (file: string, args: string[]) => Promise<Buffer>;

export class MigrationError extends Error {
  // True when the secret could not be read right now but may be readable later: the lookup tool
  // is not installed, timed out or was dismissed, or the keyring is locked or not running yet.
  // False when the answer is final: the item does not exist, or the secret does not open the blob.
  readonly retryable: boolean;

  constructor(message: string, retryable = false) {
    super(message);
    this.name = 'MigrationError';
    this.retryable = retryable;
  }
}

const SECURITY = '/usr/bin/security';
// errSecItemNotFound, as `security` reports it.
const SECURITY_ITEM_NOT_FOUND = 44;

// Only a lookup that definitely found no item is final. secret-tool exits 1 both for "no such
// item" and for a dismissed unlock prompt, and kwallet-query cannot tell a missing entry from a
// closed wallet, so on Linux every failed lookup may succeed later.
export function lookupFailureIsRetryable(file: string, code: unknown) {
  return !(file === SECURITY && code === SECURITY_ITEM_NOT_FOUND);
}

// Never includes stdout or stderr in what it rejects with: stdout is the secret.
const runCommand: CommandRunner = (file, args) =>
  new Promise((resolve, reject) => {
    execFile(
      file,
      args,
      { encoding: 'buffer', timeout: LOOKUP_TIMEOUT_MS, maxBuffer: 64 * 1024, windowsHide: true },
      (err, stdout) => {
        if (err) {
          const code = (err as any).code;
          reject(
            new MigrationError(
              `${file} failed (${code === undefined || code === null ? 'error' : code})`,
              lookupFailureIsRetryable(file, code)
            )
          );
          return;
        }
        resolve(stdout as Buffer);
      }
    );
  });

export const mailspringKeychain = {
  platform: process.platform as string,
  run: runCommand,
};

// Strips the newline the lookup tools print after the secret. Chromium's secrets are base64 text.
function trimTrailingNewline(output: Buffer) {
  let end = output.length;
  while (end > 0 && (output[end - 1] === 0x0a || output[end - 1] === 0x0d)) end--;
  return output.subarray(0, end);
}

// Candidate lookups for Mailspring's safeStorage secret, tried in order.
function lookups(platform: string): Array<[string, string[]]> {
  if (platform === 'darwin') {
    // Shows the user a keychain prompt for the "Mailspring Safe Storage" item. `-w` prints only
    // the password; nothing here can write to the keychain.
    return [[SECURITY, ['find-generic-password', '-w', '-s', `${OLD_APP_NAME} Safe Storage`]]];
  }
  if (platform === 'linux') {
    return [
      ['secret-tool', ['lookup', 'application', OLD_APP_NAME]],
      [
        'kwallet-query',
        [
          '--read-password',
          `${OLD_APP_NAME} Safe Storage`,
          '--folder',
          `${OLD_APP_NAME} Keys`,
          'kdewallet',
        ],
      ],
    ];
  }
  return [];
}

async function readOldSecret(platform: string): Promise<Buffer> {
  let lastMessage = `no keychain lookup on ${platform}`;
  // Retryable if any one lookup might succeed later.
  let retryable = false;
  for (const [file, args] of lookups(platform)) {
    let output: Buffer = null;
    try {
      output = await mailspringKeychain.run(file, args);
    } catch (err) {
      lastMessage = err instanceof MigrationError ? err.message : `${file} failed`;
      retryable = retryable || !(err instanceof MigrationError) || err.retryable;
      continue;
    }
    const secret = trimTrailingNewline(output);
    if (secret.length > 0) return secret;
    output.fill(0);
    lastMessage = `${file} returned an empty secret`;
  }
  throw new MigrationError(lastMessage, retryable);
}

export function deriveKey(secret: Buffer, platform: string): Buffer {
  return crypto.pbkdf2Sync(secret, SALT, KDF_ITERATIONS[platform], KEY_LENGTH, 'sha1');
}

export function decryptBlob(blob: Buffer, key: Buffer, platform: string): string {
  const tag = TAGS[platform];
  if (!tag || blob.subarray(0, tag.length).toString('latin1') !== tag) {
    throw new MigrationError('not a Mailspring safeStorage blob');
  }
  try {
    const decipher = crypto.createDecipheriv('aes-128-cbc', key, IV);
    return Buffer.concat([decipher.update(blob.subarray(tag.length)), decipher.final()]).toString(
      'utf8'
    );
  } catch (err) {
    // A bad padding error is the only sign of a wrong key.
    throw new MigrationError('the Mailspring key does not decrypt the saved passwords');
  }
}

export function canMigrate(platform = mailspringKeychain.platform) {
  return lookups(platform).length > 0;
}

/**
 * Decrypts a safeStorage blob written by Mailspring. Rejects with a MigrationError when the
 * secret can't be read (missing, access declined, no lookup tool) or doesn't open the blob; its
 * `retryable` says whether a later attempt might succeed.
 */
export async function decryptWithMailspringKey(blob: Buffer): Promise<string> {
  const { platform } = mailspringKeychain;
  const tag = TAGS[platform];
  if (!tag || blob.subarray(0, tag.length).toString('latin1') !== tag) {
    throw new MigrationError('not a Mailspring safeStorage blob');
  }
  const secret = await readOldSecret(platform);
  let key: Buffer;
  try {
    key = deriveKey(secret, platform);
  } finally {
    secret.fill(0);
  }
  try {
    return decryptBlob(blob, key, platform);
  } finally {
    key.fill(0);
  }
}
