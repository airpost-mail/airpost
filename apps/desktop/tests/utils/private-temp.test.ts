// SPDX-License-Identifier: GPL-3.0-only
import fs from 'fs';
import os from 'os';
import path from 'path';
import { removePrivateTempDirs, writePrivateTempFile } from '../../src/utils/private-temp';

describe('private temp files', function () {
  it('writes each file 0600 in its own 0700 directory under the temp dir, and removes them', () => {
    const a = writePrivateTempFile('same.html', 'a');
    const b = writePrivateTempFile('same.html', 'b');
    expect(path.dirname(a)).not.toBe(path.dirname(b));
    expect(path.dirname(path.dirname(a))).toBe(os.tmpdir());
    expect(fs.readFileSync(a, 'utf8')).toBe('a');
    if (process.platform !== 'win32') {
      expect(fs.statSync(path.dirname(a)).mode & 0o777).toBe(0o700);
      expect(fs.statSync(a).mode & 0o777).toBe(0o600);
    }
    removePrivateTempDirs();
    expect(fs.existsSync(path.dirname(a))).toBe(false);
    expect(fs.existsSync(path.dirname(b))).toBe(false);
  });
});
