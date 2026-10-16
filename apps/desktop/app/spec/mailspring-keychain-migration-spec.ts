import crypto from 'crypto';
import {
  decryptBlob,
  decryptWithMailspringKey,
  deriveKey,
  lookupFailureIsRetryable,
  mailspringKeychain,
  MigrationError,
} from '../src/mailspring-keychain-migration';

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

describe('mailspring-keychain-migration', function () {
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
      expect(() =>
        decryptBlob(blob, deriveKey(Buffer.from('other'), 'darwin'), 'darwin')
      ).toThrow();
    });
  });

  describe('on macOS', function () {
    beforeEach(function () {
      mailspringKeychain.platform = 'darwin';
    });

    it('reads "Mailspring Safe Storage" with security and decrypts a v10 blob', async function () {
      this.outputs.push(`${SECRET}\n`);
      const blob = chromiumBlob('{"a-imap":"secret"}', SECRET, 1003, 'v10');
      expect(await decryptWithMailspringKey(blob)).toBe('{"a-imap":"secret"}');
      expect(mailspringKeychain.run).toHaveBeenCalledWith('/usr/bin/security', [
        'find-generic-password',
        '-w',
        '-s',
        'Mailspring Safe Storage',
      ]);
    });

    it('fails for good when the item is missing', async function () {
      this.outputs.push(new MigrationError('/usr/bin/security failed (44)', false));
      const blob = chromiumBlob('{}', SECRET, 1003, 'v10');
      const err = await rejection(decryptWithMailspringKey(blob));
      expect(err instanceof MigrationError).toBe(true);
      expect(err.retryable).toBe(false);
    });

    it('can be retried when the prompt is declined or the lookup fails', async function () {
      this.outputs.push(new MigrationError('/usr/bin/security failed (51)', true));
      const blob = chromiumBlob('{}', SECRET, 1003, 'v10');
      expect((await rejection(decryptWithMailspringKey(blob))).retryable).toBe(true);

      this.outputs.push(new Error('unexpected'));
      expect((await rejection(decryptWithMailspringKey(blob))).retryable).toBe(true);
    });

    it('fails for good when the secret does not open the blob', async function () {
      this.outputs.push('wrong-secret\n');
      const blob = chromiumBlob('{}', SECRET, 1003, 'v10');
      expect((await rejection(decryptWithMailspringKey(blob))).retryable).toBe(false);
    });

    it('does not ask for the secret when the blob is not tagged v10', async function () {
      const blob = chromiumBlob('{}', SECRET, 1003, 'v11');
      expect((await rejection(decryptWithMailspringKey(blob))) instanceof MigrationError).toBe(
        true
      );
      expect(mailspringKeychain.run).not.toHaveBeenCalled();
    });

    it('never puts the secret in the error', async function () {
      this.outputs.push('wrong-secret\n');
      const blob = chromiumBlob('{}', SECRET, 1003, 'v10');
      const err = await rejection(decryptWithMailspringKey(blob));
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
      expect(await decryptWithMailspringKey(blob)).toBe('{"a-imap":"secret"}');
      expect(mailspringKeychain.run).toHaveBeenCalledWith('secret-tool', [
        'lookup',
        'application',
        'Mailspring',
      ]);
    });

    it('falls back to KWallet', async function () {
      this.outputs.push(new MigrationError('secret-tool failed (ENOENT)'), `${SECRET}\n`);
      const blob = chromiumBlob('{"a-imap":"secret"}', SECRET, 1, 'v11');
      expect(await decryptWithMailspringKey(blob)).toBe('{"a-imap":"secret"}');
      expect((mailspringKeychain.run as jasmine.Spy).calls[1].args).toEqual([
        'kwallet-query',
        ['--read-password', 'Mailspring Safe Storage', '--folder', 'Mailspring Keys', 'kdewallet'],
      ]);
    });

    it('fails, retryably, when no lookup finds the secret', async function () {
      this.outputs.push(
        new MigrationError('secret-tool failed (ENOENT)', true),
        new MigrationError('kwallet-query failed (ENOENT)', true)
      );
      const blob = chromiumBlob('{}', SECRET, 1, 'v11');
      const err = await rejection(decryptWithMailspringKey(blob));
      expect(err instanceof MigrationError).toBe(true);
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

  it('has nothing to migrate on Windows', async function () {
    mailspringKeychain.platform = 'win32';
    const blob = chromiumBlob('{}', SECRET, 1, 'v10');
    expect((await rejection(decryptWithMailspringKey(blob))) instanceof MigrationError).toBe(true);
    expect(mailspringKeychain.run).not.toHaveBeenCalled();
  });
});
