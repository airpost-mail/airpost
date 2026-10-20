import fs from 'fs';
import os from 'os';
import path from 'path';
import {
  SKIP_MAILSPRING_IMPORT_MARKER,
  shouldImportMailspringProfile,
} from '../../src/main/mailspring-profile-import';
import {
  canResetProfileDirectory,
  profileResetDetail,
  resetProfileDirectory,
} from '../../src/main/profile-reset';
import { TERN_PROFILE_MARKER, markTernProfile } from '../../src/main/profile-marker';

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
    write(path.join(tern, 'lockfile'), 'lock');
    markTernProfile(tern);
  });

  afterEach(function () {
    fs.rmSync(root, { recursive: true, force: true });
  });

  it('empties the profile except the running instance lock, and leaves the markers', function () {
    expect(resetProfileDirectory(tern)).toEqual([]);
    expect(fs.readdirSync(tern).sort()).toEqual(
      [TERN_PROFILE_MARKER, 'SingletonLock', 'lockfile', SKIP_MAILSPRING_IMPORT_MARKER].sort()
    );
    // Still marked, so it can be reset again.
    expect(canResetProfileDirectory(tern)).toBe(true);
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

  it("doesn't report Chromium storage that clearStorageData already emptied", function () {
    const realRmSync = fs.rmSync;
    spyOn(fs, 'rmSync').andCallFake((entry: string, options: fs.RmOptions) => {
      if (path.basename(entry) === 'Local Storage') throw new Error('EBUSY');
      realRmSync(entry, options);
    });
    expect(resetProfileDirectory(tern)).toEqual([]);
    expect(console.error).toHaveBeenCalled();
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

  it('refuses a directory Tern did not mark as its profile', function () {
    fs.rmSync(path.join(tern, TERN_PROFILE_MARKER));
    expect(canResetProfileDirectory(tern)).toBe(false);
    expect(() => resetProfileDirectory(tern)).toThrow();
    expect(fs.existsSync(path.join(tern, 'config.json'))).toBe(true);
  });

  it('refuses a folder that contains a protected one, even when marked', function () {
    const appData = path.join(root, 'AppData');
    write(path.join(appData, 'Other', 'x'));
    markTernProfile(root);
    expect(canResetProfileDirectory(root, [appData])).toBe(false);
    expect(canResetProfileDirectory(tern, [appData])).toBe(true);
    expect(canResetProfileDirectory(path.dirname(os.homedir()))).toBe(false);
  });

  it('refuses a link to a protected folder', function () {
    const appData = path.join(root, 'AppData');
    write(path.join(appData, TERN_PROFILE_MARKER));
    const link = path.join(root, 'link');
    fs.symlinkSync(appData, link, 'dir');
    expect(canResetProfileDirectory(link, [appData])).toBe(false);
  });

  if (process.platform === 'darwin' || process.platform === 'win32') {
    it('refuses a protected folder spelled in another case', function () {
      const appData = path.join(root, 'AppData');
      write(path.join(appData, TERN_PROFILE_MARKER));
      expect(canResetProfileDirectory(path.join(root, 'appdata'), [appData])).toBe(false);
      expect(canResetProfileDirectory(os.homedir().toUpperCase())).toBe(false);
    });
  }

  it('names the directory in the confirmation', function () {
    expect(profileResetDetail(tern)).toContain(tern);
  });
});
