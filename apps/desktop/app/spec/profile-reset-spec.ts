import fs from 'fs';
import os from 'os';
import path from 'path';
import {
  SKIP_MAILSPRING_IMPORT_MARKER,
  shouldImportMailspringProfile,
} from '../src/browser/mailspring-profile-import';
import {
  canResetProfileDirectory,
  profileResetDetail,
  resetProfileDirectory,
} from '../src/browser/profile-reset';

function write(file: string, contents = 'x') {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, contents);
}

describe('resetProfileDirectory', function () {
  let root: string;
  let mailspring: string;
  let tern: string;

  beforeEach(function () {
    spyOn(console, 'error');
    root = fs.mkdtempSync(path.join(os.tmpdir(), 'profile-reset-spec-'));
    mailspring = path.join(root, 'Mailspring');
    tern = path.join(root, 'Tern');
    write(path.join(mailspring, 'config.json'), '{"*":{"accounts":[{"id":"a"}]}}');
    write(path.join(tern, 'config.json'), '{"*":{"accounts":[{"id":"a"}]}}');
    write(path.join(tern, 'edgehill.db'), 'db');
    write(path.join(tern, 'edgehill.db-wal'), 'wal');
    write(path.join(tern, 'files', 'ab', 'cd', 'attachment.pdf'), 'pdf');
    write(path.join(tern, 'packages', 'theme', 'package.json'), '{}');
    write(path.join(tern, 'run', 'engine.json'), '{"pid":1}');
    write(path.join(tern, 'Local Storage', 'leveldb', 'CURRENT'), 'ls');
    write(path.join(tern, 'mailsync-abc.log'), 'log');
    write(path.join(tern, 'SingletonLock'), 'lock');
  });

  afterEach(function () {
    fs.rmSync(root, { recursive: true, force: true });
  });

  it('empties the profile except the running instance lock, and leaves the marker', function () {
    expect(resetProfileDirectory(tern)).toEqual([]);
    expect(fs.readdirSync(tern).sort()).toEqual(['SingletonLock', SKIP_MAILSPRING_IMPORT_MARKER]);
  });

  it('keeps the next launch from importing the Mailspring profile again', function () {
    expect(shouldImportMailspringProfile(mailspring, tern)).toBe(false);
    resetProfileDirectory(tern);
    expect(fs.existsSync(path.join(tern, 'config.json'))).toBe(false);
    expect(shouldImportMailspringProfile(mailspring, tern)).toBe(false);
    // Without the marker the empty profile would import again.
    fs.rmSync(path.join(tern, SKIP_MAILSPRING_IMPORT_MARKER));
    expect(shouldImportMailspringProfile(mailspring, tern)).toBe(true);
  });

  it('writes the marker after deleting, so an existing one survives the reset', function () {
    write(path.join(tern, SKIP_MAILSPRING_IMPORT_MARKER), 'old');
    resetProfileDirectory(tern);
    expect(fs.existsSync(path.join(tern, SKIP_MAILSPRING_IMPORT_MARKER))).toBe(true);
  });

  it('reports what it could not delete and still writes the marker', function () {
    const realRmSync = fs.rmSync;
    spyOn(fs, 'rmSync').andCallFake((entry: string, options: fs.RmOptions) => {
      if (path.basename(entry) === 'edgehill.db') throw new Error('EBUSY');
      realRmSync(entry, options);
    });
    expect(resetProfileDirectory(tern)).toEqual([path.join(tern, 'edgehill.db')]);
    expect(fs.existsSync(path.join(tern, SKIP_MAILSPRING_IMPORT_MARKER))).toBe(true);
  });

  it('refuses directories that hold more than the profile', function () {
    expect(canResetProfileDirectory(tern)).toBe(true);
    expect(canResetProfileDirectory(os.homedir())).toBe(false);
    expect(canResetProfileDirectory(`${os.homedir()}${path.sep}`)).toBe(false);
    expect(canResetProfileDirectory(path.parse(tern).root)).toBe(false);
    expect(canResetProfileDirectory(root, [root])).toBe(false);
    expect(canResetProfileDirectory('relative/Tern')).toBe(false);
    expect(canResetProfileDirectory('')).toBe(false);
    expect(() => resetProfileDirectory(root, [root])).toThrow();
    expect(fs.existsSync(path.join(tern, 'config.json'))).toBe(true);
  });

  it('names the directory in the confirmation', function () {
    expect(profileResetDetail(tern)).toContain(tern);
  });
});
