// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import fs from 'fs';
import os from 'os';
import path from 'path';
import {
  installedEngineUnit,
  runEngineUnit,
  SERVICES_FILE,
} from '../../src/engine/supervisor/unit';

describe('the engine unit', function () {
  let root: string;
  let ternBin: string | undefined;

  beforeEach(function () {
    root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'tern-engine-unit-')));
    ternBin = process.env.TERN_BIN;
  });

  afterEach(function () {
    if (ternBin === undefined) delete process.env.TERN_BIN;
    else process.env.TERN_BIN = ternBin;
    fs.rmSync(root, { recursive: true, force: true });
  });

  it('is installed only with a record and its unit file', function () {
    expect(installedEngineUnit(root)).toBe(null);
    const unit = path.join(root, 'com.ternmail.tern.engine.plist');
    const records = { engine: { name: 'com.ternmail.tern.engine', unit }, services: {} };
    fs.writeFileSync(path.join(root, SERVICES_FILE), JSON.stringify(records));
    expect(installedEngineUnit(root)).toBe(null);
    fs.writeFileSync(unit, '<plist/>');
    expect(installedEngineUnit(root)).toEqual({ name: 'com.ternmail.tern.engine', unit });
    fs.writeFileSync(path.join(root, SERVICES_FILE), '{not json');
    expect(installedEngineUnit(root)).toBe(null);
  });

  it('runs tern engine start or restart for the profile', async function () {
    // A stand-in for tern that records its arguments: no unit is touched.
    const log = path.join(root, 'args');
    const fake = path.join(root, 'tern');
    fs.writeFileSync(fake, `#!/bin/sh\necho "$@" >> "${log}"\n`);
    fs.chmodSync(fake, 0o755);
    process.env.TERN_BIN = fake;
    expect(await runEngineUnit(root, root, 'restart')).toBe(true);
    expect(await runEngineUnit(root, root, 'start')).toBe(true);
    expect(fs.readFileSync(log, 'utf8').split('\n')).toEqual([
      `--config-dir ${root} engine restart`,
      `--config-dir ${root} engine start`,
      '',
    ]);

    fs.writeFileSync(fake, '#!/bin/sh\necho "tern: no unit" >&2\nexit 1\n');
    expect(await runEngineUnit(root, root, 'start')).toBe(false);
    process.env.TERN_BIN = path.join(root, 'missing');
    expect(await runEngineUnit(root, root, 'start')).toBe(false);
  });
});
