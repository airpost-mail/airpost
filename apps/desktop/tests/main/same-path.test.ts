// SPDX-License-Identifier: GPL-3.0-only
import fs from 'fs';
import os from 'os';
import path from 'path';
import { isSamePath } from '../../src/main/same-path';

describe('isSamePath', function () {
  let root: string;

  beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), 'tern-same-path-'));
  });

  afterEach(() => {
    fs.rmSync(root, { recursive: true, force: true });
  });

  it('matches a directory reached through a symlink against its real path', () => {
    const real = path.join(root, 'real');
    fs.mkdirSync(real);
    const link = path.join(root, 'link');
    fs.symlinkSync(real, link);
    expect(isSamePath(link, fs.realpathSync(real))).toBe(true);
    expect(isSamePath(path.join(link, '.'), real)).toBe(true);
    expect(isSamePath(path.join(root, 'missing'), real)).toBe(false);
  });
});
