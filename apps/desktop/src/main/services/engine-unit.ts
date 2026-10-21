// SPDX-License-Identifier: GPL-3.0-only
import { execFile } from 'child_process';
import { devEngineKeyFile, ternBinaryPath } from '../../engine/process';
import { installedEngineUnit } from '../../engine/supervisor/unit';
import { pidAlive, readEngineDiscovery } from '../../engine/supervisor/supervisor';
import type { EngineUnitResult, EngineUnitStatus } from '../../platform/services';

/*
Preferences > General > Keep Tern running in the background: the engine as a launchd or
systemd user unit, through the bundled `tern engine install|uninstall` (apps/cli/src/service.rs).

The unit is written without being loaded (`--no-load`): its engine would find this app's
engine on the socket, exit and be restarted by the manager over and over while the app runs.
launchd loads ~/Library/LaunchAgents at the next login; on Linux the unit is enabled here
(what loading does, minus the restart). The quit path in application.ts starts the unit once
the app's own engine has stopped, and the next launch joins the unit's engine
(`detachedEngineToReuse`).

Uninstalling stops the unit's engine. When this app had joined it, the supervisor sees the
engine exit and starts its own, as after any engine crash.
*/

const TERN_TIMEOUT_MS = 30000;

export type Runner = (
  file: string,
  args: string[],
  env?: NodeJS.ProcessEnv
) => Promise<{ ok: boolean; stdout: string; stderr: string }>;

export const execRunner: Runner = (file, args, env) =>
  new Promise((resolve) => {
    execFile(
      file,
      args,
      { timeout: TERN_TIMEOUT_MS, env: { ...process.env, ...env } },
      (err, stdout, stderr) =>
        resolve({
          ok: !err,
          stdout: String(stdout || ''),
          stderr: String(stderr || '') || (err ? err.message : ''),
        })
    );
  });

const supportedPlatform = (platform: string) => platform === 'darwin' || platform === 'linux';

export function engineUnitStatus(
  configDirPath: string,
  resourcePath: string,
  { joined, platform = process.platform }: { joined: boolean; platform?: string }
): EngineUnitStatus {
  const unit = installedEngineUnit(configDirPath);
  const found = unit ? readEngineDiscovery(configDirPath) : null;
  return {
    available: supportedPlatform(platform) && !!ternBinaryPath(resourcePath),
    installed: !!unit,
    running: !!found && !!found.detached && pidAlive(found.pid),
    joined: !!unit && joined,
    unit: unit ? unit.unit : null,
  };
}

/** tern prints `error: <message>` on stderr. */
function ternError(stderr: string) {
  return stderr.trim().replace(/^error: /, '') || 'tern failed';
}

export async function setEngineUnit(
  configDirPath: string,
  resourcePath: string,
  install: boolean,
  {
    joined,
    run = execRunner,
    platform = process.platform,
  }: { joined: boolean; run?: Runner; platform?: string }
): Promise<EngineUnitResult> {
  const status = () => engineUnitStatus(configDirPath, resourcePath, { joined, platform });
  const tern = ternBinaryPath(resourcePath);
  if (!tern || !supportedPlatform(platform)) {
    return { ok: false, error: 'unsupported', status: status() };
  }
  const base = ['--config-dir', configDirPath, 'engine'];
  if (!install) {
    const result = await run(tern, [...base, 'uninstall']);
    return result.ok
      ? { ok: true, status: status() }
      : { ok: false, error: ternError(result.stderr), status: status() };
  }
  // The unit's engine has to open the secrets this app's engine sealed: in a dev run that is
  // a key file rather than the keychain (engine/process.ts).
  const keyFile = process.env.TERN_ENGINE_KEY_FILE || devEngineKeyFile(configDirPath, resourcePath);
  const args = [...base, 'install', '--no-load', ...(keyFile ? ['--key-file', keyFile] : [])];
  const result = await run(tern, args);
  if (!result.ok) return { ok: false, error: ternError(result.stderr), status: status() };
  const unit = installedEngineUnit(configDirPath);
  if (platform === 'linux' && unit) {
    await run('systemctl', ['--user', 'daemon-reload']);
    const enabled = await run('systemctl', ['--user', 'enable', `${unit.name}.service`]);
    if (!enabled.ok) return { ok: false, error: enabled.stderr.trim(), status: status() };
  }
  return { ok: true, status: status() };
}
