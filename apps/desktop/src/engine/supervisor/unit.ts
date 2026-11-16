// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

/*
The engine unit `airpost engine install` or `airpost service install` put in place (stage 19): a
launchd LaunchAgent or systemd --user unit running `serve --detached` for service plugins.
Both units restart only after a failed exit, and the engine exits 0 on `engine.shutdown`, so
an engine the desktop stopped would stay stopped once the desktop quits. Instead the launch
restarts the unit when its engine runs another build (after an update the unit's path holds
the new one) and joins it, and the quit starts the unit again after the desktop's own engine
exited. Both go through the bundled `airpost`, which never loads a unit outside the manager's own
directory or under AIRPOST_SERVICE_NO_LOAD.
*/
import fs from 'fs';
import path from 'path';
import { execFile } from 'child_process';
import { airpostBinaryPath } from '../process';

// Written by airpost (apps/cli/src/service.rs, RECORDS_FILE).
export const SERVICES_FILE = 'services.json';
const AIRPOST_TIMEOUT_MS = 15000;

export interface EngineUnit {
  name: string;
  unit: string;
}

// The engine unit services.json records, when its unit file is still there.
export function installedEngineUnit(configDirPath: string): EngineUnit | null {
  try {
    const records = JSON.parse(fs.readFileSync(path.join(configDirPath, SERVICES_FILE)).toString());
    const engine = records && records.engine;
    if (!engine || typeof engine.name !== 'string' || typeof engine.unit !== 'string') return null;
    return fs.existsSync(engine.unit) ? { name: engine.name, unit: engine.unit } : null;
  } catch (err) {
    return null;
  }
}

// `airpost engine start|restart` for this profile; resolves false (and logs why) when it failed.
export function runEngineUnit(
  configDirPath: string,
  resourcePath: string,
  action: 'start' | 'restart',
  timeoutMs = AIRPOST_TIMEOUT_MS
): Promise<boolean> {
  const airpost = airpostBinaryPath(resourcePath);
  if (!airpost) {
    console.warn(`Could not ${action} the engine unit: airpost is not in this build`);
    return Promise.resolve(false);
  }
  return new Promise((resolve) => {
    execFile(
      airpost,
      ['--config-dir', configDirPath, 'engine', action],
      { timeout: timeoutMs },
      (err, _stdout, stderr) => {
        if (err) {
          const why = String(stderr || '').trim() || err.message;
          console.warn(`airpost engine ${action} failed: ${why}`);
        }
        resolve(!err);
      }
    );
  });
}
