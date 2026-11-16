// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

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
  let airpostBin: string | undefined;

  beforeEach(function () {
    root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'airpost-engine-unit-')));
    airpostBin = process.env.AIRPOST_BIN;
  });

  afterEach(function () {
    if (airpostBin === undefined) delete process.env.AIRPOST_BIN;
    else process.env.AIRPOST_BIN = airpostBin;
    fs.rmSync(root, { recursive: true, force: true });
  });

  it('is installed only with a record and its unit file', function () {
    expect(installedEngineUnit(root)).toBe(null);
    const unit = path.join(root, 'com.airpost.airpost.engine.plist');
    const records = { engine: { name: 'com.airpost.airpost.engine', unit }, services: {} };
    fs.writeFileSync(path.join(root, SERVICES_FILE), JSON.stringify(records));
    expect(installedEngineUnit(root)).toBe(null);
    fs.writeFileSync(unit, '<plist/>');
    expect(installedEngineUnit(root)).toEqual({ name: 'com.airpost.airpost.engine', unit });
    fs.writeFileSync(path.join(root, SERVICES_FILE), '{not json');
    expect(installedEngineUnit(root)).toBe(null);
  });

  it('runs airpost engine start or restart for the profile', async function () {
    // A stand-in for airpost that records its arguments: no unit is touched.
    const log = path.join(root, 'args');
    const fake = path.join(root, 'airpost');
    fs.writeFileSync(fake, `#!/bin/sh\necho "$@" >> "${log}"\n`);
    fs.chmodSync(fake, 0o755);
    process.env.AIRPOST_BIN = fake;
    expect(await runEngineUnit(root, root, 'restart')).toBe(true);
    expect(await runEngineUnit(root, root, 'start')).toBe(true);
    expect(fs.readFileSync(log, 'utf8').split('\n')).toEqual([
      `--config-dir ${root} engine restart`,
      `--config-dir ${root} engine start`,
      '',
    ]);

    fs.writeFileSync(fake, '#!/bin/sh\necho "airpost: no unit" >&2\nexit 1\n');
    expect(await runEngineUnit(root, root, 'start')).toBe(false);
    process.env.AIRPOST_BIN = path.join(root, 'missing');
    expect(await runEngineUnit(root, root, 'start')).toBe(false);
  });
});
