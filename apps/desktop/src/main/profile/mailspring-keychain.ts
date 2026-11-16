import crypto from 'crypto';
import fs from 'fs';
import path from 'path';
import { execFile } from 'child_process';

/**
 * Reads the passwords Mailspring saved, for Import from Mailspring
 * (main/profile/mailspring-import.ts).
 *
 * Electron's safeStorage keys its encryption off an OS secret named after the app. Airpost's
 * secret is "Airpost Safe Storage", so the keyset Mailspring encrypted, under "Mailspring Safe
 * Storage", does not open through safeStorage. This module reads Mailspring's secret,
 * read-only, and decrypts the keyset itself, the way Chromium's os_crypt_async does:
 *
 * - macOS (KeychainKeyProvider): tag "v10", key = PBKDF2-HMAC-SHA1(keychain password,
 *   "saltysalt", 1003 iterations, 16 bytes).
 * - Linux (FreedesktopSecretKeyProvider): tag "v11", same derivation with 1 iteration, from the
 *   Secret Service item whose "application" attribute is the app name (KWallet: entry
 *   "<name> Safe Storage" in folder "<name> Keys"). A "v10" blob on Linux uses the constant
 *   password "peanuts" (PosixKeyProvider), so it needs no lookup.
 * - Both: AES-128-CBC, PKCS#7 padding, IV of 16 spaces (os_crypt_async/common/encryptor.cc).
 * - Windows (DPAPIKeyProvider): tag "v10", then a 12-byte nonce, the AES-256-GCM ciphertext and
 *   its 16-byte tag. The key is os_crypt.encrypted_key in the profile's "Local State": "DPAPI"
 *   and the key wrapped by CryptProtectData for the Windows user, which PowerShell unwraps.
 *
 * Mailspring's secret is only ever read, and is zeroed as soon as the key is derived.
 */

const OLD_APP_NAME = 'Mailspring';
const SALT = 'saltysalt';
const KEY_LENGTH = 16;
const IV = Buffer.alloc(16, ' ');
const KDF_ITERATIONS: { [platform: string]: number } = { darwin: 1003, linux: 1 };
const TAGS: { [platform: string]: string } = { darwin: 'v10', linux: 'v11', win32: 'v10' };
const LINUX_FALLBACK_TAG = 'v10';
const LINUX_FALLBACK_PASSWORD = 'peanuts';

const LOCAL_STATE_FILE = 'Local State';
const DPAPI_PREFIX = 'DPAPI';
const GCM_NONCE_LENGTH = 12;
const GCM_TAG_LENGTH = 16;
const GCM_KEY_LENGTH = 32;

// Leaves time to answer the keychain prompt, but does not wait forever on a hung helper.
const LOOKUP_TIMEOUT_MS = 5 * 60 * 1000;

export type CommandRunner = (file: string, args: string[]) => Promise<Buffer>;

// The keyset as KeyManager stores it: `<email>-imap`, `<email>-smtp`, `<email>-refresh-token`
// and the other saved secrets.
export interface MailspringKeySet {
  [key: string]: string;
}

export class MailspringKeychainError extends Error {
  // True when the secret could not be read right now but may be readable later: the lookup tool
  // is not installed, timed out or was dismissed, or the keyring is locked or not running yet.
  // False when the answer is final: the item does not exist, or the secret does not open the blob.
  readonly retryable: boolean;

  constructor(message: string, retryable = false) {
    super(message);
    this.name = 'MailspringKeychainError';
    this.retryable = retryable;
  }
}

const SECURITY = '/usr/bin/security';
// errSecItemNotFound, as `security` reports it.
const SECURITY_ITEM_NOT_FOUND = 44;
const POWERSHELL = 'powershell.exe';

// Only a lookup that definitely found no item is final. secret-tool exits 1 both for "no such
// item" and for a dismissed unlock prompt, and kwallet-query cannot tell a missing entry from a
// closed wallet, so on Linux every failed lookup may succeed later. DPAPI refuses only a key
// wrapped for another Windows user or machine, which stays so; PowerShell missing or timing out
// may not.
export function lookupFailureIsRetryable(file: string, code: unknown) {
  if (file === POWERSHELL) return code === 'ENOENT' || code === undefined || code === null;
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
            new MailspringKeychainError(
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
    let output: Buffer;
    try {
      output = await mailspringKeychain.run(file, args);
    } catch (err) {
      lastMessage = err instanceof MailspringKeychainError ? err.message : `${file} failed`;
      retryable = retryable || !(err instanceof MailspringKeychainError) || err.retryable;
      continue;
    }
    const secret = trimTrailingNewline(output);
    if (secret.length > 0) return secret;
    output.fill(0);
    lastMessage = `${file} returned an empty secret`;
  }
  throw new MailspringKeychainError(lastMessage, retryable);
}

export function deriveKey(secret: Buffer, platform: string): Buffer {
  return crypto.pbkdf2Sync(secret, SALT, KDF_ITERATIONS[platform], KEY_LENGTH, 'sha1');
}

function hasTag(blob: Buffer, tag: string) {
  return !!tag && blob.subarray(0, tag.length).toString('latin1') === tag;
}

function notABlob() {
  return new MailspringKeychainError('not a Mailspring safeStorage blob');
}

function wrongKey() {
  return new MailspringKeychainError('the Mailspring key does not decrypt the saved passwords');
}

export function decryptBlob(blob: Buffer, key: Buffer, tag: string): string {
  if (!hasTag(blob, tag)) throw notABlob();
  try {
    const decipher = crypto.createDecipheriv('aes-128-cbc', key, IV);
    return Buffer.concat([decipher.update(blob.subarray(tag.length)), decipher.final()]).toString(
      'utf8'
    );
  } catch (err) {
    // A bad padding error is the only sign of a wrong key.
    throw wrongKey();
  }
}

export function decryptWindowsBlob(blob: Buffer, key: Buffer): string {
  const tag = TAGS.win32;
  if (!hasTag(blob, tag) || blob.length < tag.length + GCM_NONCE_LENGTH + GCM_TAG_LENGTH) {
    throw notABlob();
  }
  const nonce = blob.subarray(tag.length, tag.length + GCM_NONCE_LENGTH);
  const ciphertext = blob.subarray(tag.length + GCM_NONCE_LENGTH, blob.length - GCM_TAG_LENGTH);
  try {
    const decipher = crypto.createDecipheriv('aes-256-gcm', key, nonce);
    decipher.setAuthTag(blob.subarray(blob.length - GCM_TAG_LENGTH));
    return Buffer.concat([decipher.update(ciphertext), decipher.final()]).toString('utf8');
  } catch (err) {
    throw wrongKey();
  }
}

// The AES key Mailspring's Chromium keeps in its Local State, unwrapped for this Windows user.
async function readWindowsKey(profileDir: string): Promise<Buffer> {
  let encoded: unknown = null;
  try {
    const localState = JSON.parse(
      fs.readFileSync(path.join(profileDir, LOCAL_STATE_FILE)).toString()
    );
    encoded = localState && localState.os_crypt && localState.os_crypt.encrypted_key;
  } catch (err) {
    throw new MailspringKeychainError(`Mailspring's ${LOCAL_STATE_FILE} could not be read`);
  }
  const wrapped = typeof encoded === 'string' ? Buffer.from(encoded, 'base64') : null;
  if (!wrapped || !hasTag(wrapped, DPAPI_PREFIX)) {
    throw new MailspringKeychainError(
      `Mailspring's ${LOCAL_STATE_FILE} holds no key for the saved passwords`
    );
  }
  // The literal is base64, so it can't end the quoted string early. It opens only for this user.
  const script =
    'Add-Type -AssemblyName System.Security; [Convert]::ToBase64String(' +
    "[Security.Cryptography.ProtectedData]::Unprotect([Convert]::FromBase64String('" +
    wrapped.subarray(DPAPI_PREFIX.length).toString('base64') +
    "'), $null, 'CurrentUser'))";
  const output = await mailspringKeychain.run(POWERSHELL, [
    '-NoProfile',
    '-NonInteractive',
    '-Command',
    script,
  ]);
  const key = Buffer.from(trimTrailingNewline(output).toString('latin1'), 'base64');
  output.fill(0);
  if (key.length !== GCM_KEY_LENGTH) {
    key.fill(0);
    throw new MailspringKeychainError('DPAPI returned no key for the saved passwords');
  }
  return key;
}

/**
 * Decrypts a safeStorage blob written by the Mailspring whose profile is `profileDir`. Rejects
 * with a MailspringKeychainError when the secret can't be read (missing, access declined, no
 * lookup tool) or doesn't open the blob; its `retryable` says whether a later attempt might
 * succeed.
 */
export async function decryptWithMailspringKey(blob: Buffer, profileDir: string): Promise<string> {
  const { platform } = mailspringKeychain;
  if (platform === 'win32') {
    if (!hasTag(blob, TAGS.win32)) throw notABlob();
    const key = await readWindowsKey(profileDir);
    try {
      return decryptWindowsBlob(blob, key);
    } finally {
      key.fill(0);
    }
  }
  if (platform === 'linux' && hasTag(blob, LINUX_FALLBACK_TAG)) {
    const key = deriveKey(Buffer.from(LINUX_FALLBACK_PASSWORD), platform);
    return decryptBlob(blob, key, LINUX_FALLBACK_TAG);
  }
  const tag = TAGS[platform];
  if (!hasTag(blob, tag) || !lookups(platform).length) throw notABlob();
  const secret = await readOldSecret(platform);
  let key: Buffer;
  try {
    key = deriveKey(secret, platform);
  } finally {
    secret.fill(0);
  }
  try {
    return decryptBlob(blob, key, tag);
  } finally {
    key.fill(0);
  }
}

/**
 * The keyset in Mailspring's config.json `credentials` (a serialized Buffer, as config.json
 * holds it), decrypted. Resolves to {} when Mailspring saved none.
 */
export async function readMailspringKeySet(
  stored: unknown,
  profileDir: string
): Promise<MailspringKeySet> {
  if (stored === undefined || stored === null || stored === 'null') return {};
  const blob = Buffer.from(stored as any, 'utf-8');
  const plaintext = await decryptWithMailspringKey(blob, profileDir);
  let keys: unknown = null;
  try {
    keys = JSON.parse(plaintext);
  } catch (err) {
    // A wrong key passes the padding check about once in 256 tries and yields garbage.
    throw wrongKey();
  }
  if (!keys || typeof keys !== 'object' || Array.isArray(keys)) throw wrongKey();
  return keys as MailspringKeySet;
}
