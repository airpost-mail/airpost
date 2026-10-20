import { execFile } from 'child_process';
import { devEngineKeyFile, engineBinaryPath, ternBinaryPath } from '../../../../engine/process';

// The bundled `tern` CLI (apps/cli), shipped next to mailsync-rs in app.asar.unpacked.
// Service plugins are installed and removed by `tern service install|uninstall`, so the
// desktop never writes a launchd or systemd unit itself (docs/desktop-client-plan.md T19).

export interface RunResult {
  ok: boolean;
  stdout: string;
  stderr: string;
}

export interface UnitPlan {
  name: string;
  path: string;
  text: string;
}

/** `tern --json service install --dry-run`: what an install would write and run. */
export interface ServicePlan {
  package: string;
  source: string;
  bin: string;
  sha256: string;
  // Digest of the hash and every unit text and path the install writes; tern refuses an
  // install whose plan is no longer this one.
  planSha256: string;
  installedBin: string;
  args: string[];
  manager: 'launchd' | 'systemd';
  load: boolean;
  installed: boolean;
  unit: UnitPlan;
  engine: UnitPlan & { installed: boolean; bin?: string; sha256?: string };
}

export interface ServiceRecord {
  package: string;
  source: string;
  sourceBin: string;
  bin: string;
  args: string[];
  sha256: string;
  manager: string;
  name: string;
  unit: string;
  installedAt: number;
  unitPresent: boolean;
  binUnchanged: boolean;
}

export interface ServiceList {
  engine: { name: string; unit: string; bin: string; unitPresent: boolean } | null;
  services: ServiceRecord[];
}

/** Where units go. Specs always give `unitDir`, which makes tern load nothing. */
export interface UnitOptions {
  unitDir?: string;
}

/** `TERN_BIN` (a source checkout), else the binary in this build. */
export function ternBinary(): string | null {
  return ternBinaryPath(AppEnv.getLoadSettings().resourcePath);
}

export function runTern(args: string[], timeout = 60000): Promise<RunResult> {
  const writesUnits = args.includes('install') || args.includes('uninstall');
  if (AppEnv.inSpecMode() && writesUnits && !args.includes('--unit-dir')) {
    return Promise.reject(new Error('specs must give tern service install --unit-dir'));
  }
  const binary = ternBinary();
  if (!binary) return Promise.reject(new Error('tern is not in this build of Tern'));
  return new Promise((resolve) => {
    execFile(binary, args, { timeout, maxBuffer: 4 * 1024 * 1024 }, (err, stdout, stderr) => {
      resolve({ ok: !err, stdout: String(stdout || ''), stderr: String(stderr || '') });
    });
  });
}

/** tern prints errors on stderr as `tern: <message>`. */
export function errorMessage(result: RunResult) {
  const text = (result.stderr || result.stdout).trim();
  return text.replace(/^tern: /gm, '');
}

async function runJSON<T>(args: string[]): Promise<T> {
  const result = await runTern(['--json', '--config-dir', AppEnv.getConfigDirPath(), ...args]);
  if (!result.ok) throw new Error(errorMessage(result));
  return JSON.parse(result.stdout);
}

function unitArgs({ unitDir }: UnitOptions) {
  return unitDir ? ['--unit-dir', unitDir] : [];
}

/** The engine the unit runs is this build's, with the secret store its own engine uses. */
function engineArgs() {
  const { resourcePath } = AppEnv.getLoadSettings();
  const args = ['--engine-bin', engineBinaryPath(resourcePath)];
  const keyFile =
    process.env.TERN_ENGINE_KEY_FILE || devEngineKeyFile(AppEnv.getConfigDirPath(), resourcePath);
  if (keyFile) args.push('--engine-key-file', keyFile);
  return args;
}

export function planServiceInstall(pkg: string, opts: UnitOptions = {}) {
  return runJSON<ServicePlan>([
    'service',
    'install',
    pkg,
    '--dry-run',
    ...engineArgs(),
    ...unitArgs(opts),
  ]);
}

/**
 * Installs what the plan showed: tern refuses when the binary's hash or anything it would
 * write (args, unit texts, the engine unit) is no longer what `plan` holds.
 */
export function installService(plan: ServicePlan, opts: UnitOptions = {}) {
  return runJSON<ServicePlan>([
    'service',
    'install',
    plan.package,
    '--expect-sha256',
    plan.sha256,
    '--expect-plan',
    plan.planSha256,
    ...engineArgs(),
    ...unitArgs(opts),
  ]);
}

export function uninstallService(pkg: string, opts: UnitOptions = {}) {
  return runJSON<{ removed: boolean }>(['service', 'uninstall', pkg, ...unitArgs(opts)]);
}

export function listServices() {
  return runJSON<ServiceList>(['service', 'list']);
}
