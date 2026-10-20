// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The engine unit `tern engine install` or `tern service install` put in place (stage 19): a
launchd LaunchAgent or systemd --user unit running `serve --detached` for service plugins.
Both units restart only after a failed exit, and the engine exits 0 on `engine.shutdown`, so
an engine the desktop stopped would stay stopped once the desktop quits. Instead the launch
restarts the unit when its engine runs another build (after an update the unit's path holds
the new one) and joins it, and the quit starts the unit again after the desktop's own engine
exited. Both go through the bundled `tern`, which never loads a unit outside the manager's own
directory or under TERN_SERVICE_NO_LOAD.
*/
import fs from 'fs';
import path from 'path';
import { execFile } from 'child_process';
import { ternBinaryPath } from '../shared/mailsync-process';

// Written by tern (apps/cli/src/service.rs, RECORDS_FILE).
export const SERVICES_FILE = 'services.json';
const TERN_TIMEOUT_MS = 15000;

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

// `tern engine start|restart` for this profile; resolves false (and logs why) when it failed.
export function runEngineUnit(
  configDirPath: string,
  resourcePath: string,
  action: 'start' | 'restart',
  timeoutMs = TERN_TIMEOUT_MS
): Promise<boolean> {
  const tern = ternBinaryPath(resourcePath);
  if (!tern) {
    console.warn(`Could not ${action} the engine unit: tern is not in this build`);
    return Promise.resolve(false);
  }
  return new Promise((resolve) => {
    execFile(
      tern,
      ['--config-dir', configDirPath, 'engine', action],
      { timeout: timeoutMs },
      (err, _stdout, stderr) => {
        if (err) {
          const why = String(stderr || '').trim() || err.message;
          console.warn(`tern engine ${action} failed: ${why}`);
        }
        resolve(!err);
      }
    );
  });
}
