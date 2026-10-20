import crypto from 'crypto';
import fs from 'fs';
import os from 'os';
import path from 'path';
import {
  decryptBlob,
  decryptWithMailspringKey,
  deriveKey,
  lookupFailureIsRetryable,
  mailspringKeychain,
  MailspringKeychainError,
  readMailspringKeySet,
} from '../../src/main/profile/mailspring-keychain';

// Synthetic secrets only: these specs never reach a real keychain.
const SECRET = 'c3ludGhldGljLW1haWxzcHJpbmc=';

const chromiumBlob = (plaintext: string, secret: string, iterations: number, tag: string) => {
  const key = crypto.pbkdf2Sync(secret, 'saltysalt', iterations, 16, 'sha1');
  const cipher = crypto.createCipheriv('aes-128-cbc', key, Buffer.alloc(16, ' '));
  return Buffer.concat([Buffer.from(tag), cipher.update(plaintext, 'utf8'), cipher.final()]);
};

const rejection = async (promise: Promise<any>) => {
  try {
    await promise;
  } catch (err) {
    return err;
  }
  return null;
};

// Only the Windows specs read a profile (its Local State); the others never touch it.
const PROFILE = path.join(os.tmpdir(), 'tern-spec-no-mailspring-profile');

// Encrypts the way Chromium's DPAPIKeyProvider does: "v10", nonce, AES-256-GCM, tag.
const windowsBlob = (plaintext: string, key: Buffer) => {
  const nonce = crypto.randomBytes(12);
  const cipher = crypto.createCipheriv('aes-256-gcm', key, nonce);
  const ciphertext = Buffer.concat([cipher.update(plaintext, 'utf8'), cipher.final()]);
  return Buffer.concat([Buffer.from('v10'), nonce, ciphertext, cipher.getAuthTag()]);
};

describe('mailspring-keychain', function () {
  beforeEach(function () {
    this.platform = mailspringKeychain.platform;
    this.outputs = [];
    spyOn(mailspringKeychain, 'run').andCallFake(() => {
      const next = this.outputs.shift();
      return next instanceof Error ? Promise.reject(next) : Promise.resolve(Buffer.from(next));
    });
  });

  afterEach(function () {
    mailspringKeychain.platform = this.platform;
  });

  // Constants from Chromium's os_crypt_async, which pin the derivation independently of the
  // code under test.
  describe('deriveKey', function () {
    it('matches the empty-password fallback key (PBKDF2, 1 iteration)', function () {
      expect(deriveKey(Buffer.alloc(0), 'linux').toString('hex')).toBe(
        'd0d0ec9c7d77d43ac54187fa4818d17f'
      );
    });

    it('matches the "peanuts" v10 key of PosixKeyProvider', function () {
      expect(deriveKey(Buffer.from('peanuts'), 'linux').toString('hex')).toBe(
        'fd621fe5a2b402539dfa147ca9272778'
      );
    });
  });

  describe('decryptBlob', function () {
    it('rejects a wrong key', function () {
      const blob = chromiumBlob('{"a":"b"}', SECRET, 1003, 'v10');
      expect(() => decryptBlob(blob, deriveKey(Buffer.from('other'), 'darwin'), 'v10')).toThrow();
    });
  });

  describe('on macOS', function () {
    beforeEach(function () {
      mailspringKeychain.platform = 'darwin';
    });

    it('reads "Mailspring Safe Storage" with security and decrypts a v10 blob', async function () {
      this.outputs.push(`${SECRET}\n`);
      const blob = chromiumBlob('{"a-imap":"secret"}', SECRET, 1003, 'v10');
      expect(await decryptWithMailspringKey(blob, PROFILE)).toBe('{"a-imap":"secret"}');
      expect(mailspringKeychain.run).toHaveBeenCalledWith('/usr/bin/security', [
        'find-generic-password',
        '-w',
        '-s',
        'Mailspring Safe Storage',
      ]);
    });

    it('fails for good when the item is missing', async function () {
      this.outputs.push(new MailspringKeychainError('/usr/bin/security failed (44)', false));
      const blob = chromiumBlob('{}', SECRET, 1003, 'v10');
      const err = await rejection(decryptWithMailspringKey(blob, PROFILE));
      expect(err instanceof MailspringKeychainError).toBe(true);
      expect(err.retryable).toBe(false);
    });

    it('can be retried when the prompt is declined or the lookup fails', async function () {
      this.outputs.push(new MailspringKeychainError('/usr/bin/security failed (51)', true));
      const blob = chromiumBlob('{}', SECRET, 1003, 'v10');
      expect((await rejection(decryptWithMailspringKey(blob, PROFILE))).retryable).toBe(true);

      this.outputs.push(new Error('unexpected'));
      expect((await rejection(decryptWithMailspringKey(blob, PROFILE))).retryable).toBe(true);
    });

    it('fails for good when the secret does not open the blob', async function () {
      this.outputs.push('wrong-secret\n');
      const blob = chromiumBlob('{}', SECRET, 1003, 'v10');
      expect((await rejection(decryptWithMailspringKey(blob, PROFILE))).retryable).toBe(false);
    });

    it('does not ask for the secret when the blob is not tagged v10', async function () {
      const blob = chromiumBlob('{}', SECRET, 1003, 'v11');
      expect(
        (await rejection(decryptWithMailspringKey(blob, PROFILE))) instanceof
          MailspringKeychainError
      ).toBe(true);
      expect(mailspringKeychain.run).not.toHaveBeenCalled();
    });

    it('never puts the secret in the error', async function () {
      this.outputs.push('wrong-secret\n');
      const blob = chromiumBlob('{}', SECRET, 1003, 'v10');
      const err = await rejection(decryptWithMailspringKey(blob, PROFILE));
      expect(err.message).not.toContain('wrong-secret');
      expect(err.message).not.toContain(SECRET);
    });
  });

  describe('on Linux', function () {
    beforeEach(function () {
      mailspringKeychain.platform = 'linux';
    });

    it('reads the Secret Service item and decrypts a v11 blob', async function () {
      this.outputs.push(SECRET);
      const blob = chromiumBlob('{"a-imap":"secret"}', SECRET, 1, 'v11');
      expect(await decryptWithMailspringKey(blob, PROFILE)).toBe('{"a-imap":"secret"}');
      expect(mailspringKeychain.run).toHaveBeenCalledWith('secret-tool', [
        'lookup',
        'application',
        'Mailspring',
      ]);
    });

    it('falls back to KWallet', async function () {
      this.outputs.push(new MailspringKeychainError('secret-tool failed (ENOENT)'), `${SECRET}\n`);
      const blob = chromiumBlob('{"a-imap":"secret"}', SECRET, 1, 'v11');
      expect(await decryptWithMailspringKey(blob, PROFILE)).toBe('{"a-imap":"secret"}');
      expect((mailspringKeychain.run as jasmine.Spy).calls[1].args).toEqual([
        'kwallet-query',
        ['--read-password', 'Mailspring Safe Storage', '--folder', 'Mailspring Keys', 'kdewallet'],
      ]);
    });

    it('fails, retryably, when no lookup finds the secret', async function () {
      this.outputs.push(
        new MailspringKeychainError('secret-tool failed (ENOENT)', true),
        new MailspringKeychainError('kwallet-query failed (ENOENT)', true)
      );
      const blob = chromiumBlob('{}', SECRET, 1, 'v11');
      const err = await rejection(decryptWithMailspringKey(blob, PROFILE));
      expect(err instanceof MailspringKeychainError).toBe(true);
      expect(err.retryable).toBe(true);
    });
  });

  describe('lookupFailureIsRetryable', function () {
    it('is final only when security reports that the item does not exist', function () {
      expect(lookupFailureIsRetryable('/usr/bin/security', 44)).toBe(false);
      expect(lookupFailureIsRetryable('/usr/bin/security', 51)).toBe(true);
      expect(lookupFailureIsRetryable('/usr/bin/security', 128)).toBe(true);
      expect(lookupFailureIsRetryable('/usr/bin/security', 'ENOENT')).toBe(true);
      // A timeout kills the child, which leaves no exit code.
      expect(lookupFailureIsRetryable('/usr/bin/security', null)).toBe(true);
    });

    it('never treats a Linux lookup failure as final', function () {
      expect(lookupFailureIsRetryable('secret-tool', 1)).toBe(true);
      expect(lookupFailureIsRetryable('secret-tool', 'ENOENT')).toBe(true);
      expect(lookupFailureIsRetryable('kwallet-query', 1)).toBe(true);
    });
  });

  describe('on Windows', function () {
    const KEY = Buffer.alloc(32, 7);
    let profile: string;

    beforeEach(function () {
      mailspringKeychain.platform = 'win32';
      profile = fs.mkdtempSync(path.join(os.tmpdir(), 'mailspring-keychain-spec-'));
      const wrapped = Buffer.concat([Buffer.from('DPAPI'), Buffer.from('wrapped-for-this-user')]);
      fs.writeFileSync(
        path.join(profile, 'Local State'),
        JSON.stringify({ os_crypt: { encrypted_key: wrapped.toString('base64') } })
      );
    });

    afterEach(function () {
      fs.rmSync(profile, { recursive: true, force: true });
    });

    it('unwraps the key in Local State with DPAPI and decrypts a v10 blob', async function () {
      this.outputs.push(`${KEY.toString('base64')}\r\n`);
      const blob = windowsBlob('{"a-imap":"secret"}', KEY);
      expect(await decryptWithMailspringKey(blob, profile)).toBe('{"a-imap":"secret"}');
      const [file, args] = (mailspringKeychain.run as jasmine.Spy).calls[0].args;
      expect(file).toBe('powershell.exe');
      // The wrapped key without its "DPAPI" prefix, base64 inside the script.
      expect(args[args.length - 1]).toContain(
        Buffer.from('wrapped-for-this-user').toString('base64')
      );
      expect(args[args.length - 1]).toContain("'CurrentUser'");
    });

    it('fails for good when the key does not open the blob', async function () {
      this.outputs.push(Buffer.alloc(32, 9).toString('base64'));
      const err = await rejection(decryptWithMailspringKey(windowsBlob('{}', KEY), profile));
      expect(err instanceof MailspringKeychainError).toBe(true);
      expect(err.retryable).toBe(false);
    });

    it('fails without running anything when Local State has no key', async function () {
      fs.writeFileSync(path.join(profile, 'Local State'), '{}');
      const err = await rejection(decryptWithMailspringKey(windowsBlob('{}', KEY), profile));
      expect(err instanceof MailspringKeychainError).toBe(true);
      expect(mailspringKeychain.run).not.toHaveBeenCalled();
    });
  });

  describe('on Linux, a v10 blob', function () {
    it('opens with the constant "peanuts" key, with no lookup', async function () {
      mailspringKeychain.platform = 'linux';
      const blob = chromiumBlob('{"a-imap":"secret"}', 'peanuts', 1, 'v10');
      expect(await decryptWithMailspringKey(blob, PROFILE)).toBe('{"a-imap":"secret"}');
      expect(mailspringKeychain.run).not.toHaveBeenCalled();
    });
  });

  describe('readMailspringKeySet', function () {
    beforeEach(function () {
      mailspringKeychain.platform = 'darwin';
    });

    it('decrypts the credentials as config.json stores them', async function () {
      this.outputs.push(`${SECRET}\n`);
      const stored = chromiumBlob('{"a@example.com-imap":"secret"}', SECRET, 1003, 'v10').toJSON();
      expect(await readMailspringKeySet(stored, PROFILE)).toEqual({
        'a@example.com-imap': 'secret',
      });
    });

    it('is empty, with no lookup, when Mailspring saved nothing', async function () {
      for (const stored of [undefined, null, 'null']) {
        expect(await readMailspringKeySet(stored, PROFILE)).toEqual({});
      }
      expect(mailspringKeychain.run).not.toHaveBeenCalled();
    });

    it('rejects a blob that decrypts to something that is not a keyset', async function () {
      this.outputs.push(`${SECRET}\n`);
      const stored = chromiumBlob('["not", "a", "keyset"]', SECRET, 1003, 'v10').toJSON();
      expect(
        (await rejection(readMailspringKeySet(stored, PROFILE))) instanceof MailspringKeychainError
      ).toBe(true);
    });
  });
});
