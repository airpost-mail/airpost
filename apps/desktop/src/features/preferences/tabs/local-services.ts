import {
  devEngineKeyFile,
  engineBinaryPath,
  ternGatewayBinaryPath,
  ternWebRootPath,
} from '../../../engine/process';
import { localized } from '../../../intl/localize';
import type { StatusKind } from '../../../components/settings';
import { RunResult, errorMessage, runTern } from './plugins/tern-cli';

/*
The services next to the app (Preferences > General > Services on this computer, and the MCP
tab's HTTP server): the engine, tern-gateway and `tern-mcp http`, each a launchd or systemd
user unit that the bundled `tern` installs, starts and removes (`tern <service> install|
uninstall|start|stop`) and reports in `tern --json status`. The desktop only runs those
commands, so a service behaves the same set up from a terminal and keeps running after the
app quits.
*/

export type LocalServiceName = 'engine' | 'gateway' | 'mcp-http';

/** One entry of `tern --json status`'s `services`. */
export interface LocalService {
  name: LocalServiceName;
  installed: boolean;
  running: boolean;
  pid: number | null;
  url: string | null;
  port: number | null;
  unit: string | null;
  bin: string | null;
}

export type Runner = (args: string[], timeout?: number) => Promise<RunResult>;

export interface ServiceStatuses {
  /** Null when tern couldn't say: not in this build, or too old to list services. */
  services: LocalService[] | null;
  error: string | null;
}

export const GATEWAY_BIND = '127.0.0.1:5180';

function base() {
  return ['--json', '--config-dir', AppEnv.getConfigDirPath()];
}

/** `tern --json status` exits 3 with no engine running and still prints the services. */
export async function serviceStatuses(run: Runner = runTern): Promise<ServiceStatuses> {
  let result: RunResult;
  try {
    result = await run([...base(), 'status'], 20000);
  } catch (err) {
    return { services: null, error: (err as Error).message };
  }
  try {
    const parsed = JSON.parse(result.stdout);
    if (Array.isArray(parsed?.services)) return { services: parsed.services, error: null };
  } catch (err) {
    // not JSON: an error message, or an older tern
  }
  return {
    services: null,
    error: result.ok ? localized('This tern doesn’t list services yet.') : errorMessage(result),
  };
}

export function findService(statuses: ServiceStatuses, name: LocalServiceName) {
  return statuses.services?.find((s) => s.name === name) || null;
}

/** What this build lacks to run the web client: tern-gateway, or the web app it serves. */
export function webAccessMissing() {
  const { resourcePath } = AppEnv.getLoadSettings();
  const missing: string[] = [];
  if (!ternGatewayBinaryPath(resourcePath)) missing.push('tern-gateway');
  if (!ternWebRootPath(resourcePath)) missing.push('web');
  return missing;
}

/**
 * The engine unit runs this build's engine with the secret store this app's engine uses (a
 * key file in a dev run). It is enabled for login but not started (`--no-start`): started now,
 * its engine would find the app's on the socket, exit and be restarted by the manager while
 * the app runs. It starts at the next login, and the app starts it when it quits.
 */
function installArgs(name: LocalServiceName, opts: { port?: number }) {
  const { resourcePath } = AppEnv.getLoadSettings();
  if (name === 'engine') {
    const keyFile =
      process.env.TERN_ENGINE_KEY_FILE || devEngineKeyFile(AppEnv.getConfigDirPath(), resourcePath);
    return [
      '--bin',
      engineBinaryPath(resourcePath),
      '--no-start',
      ...(keyFile ? ['--key-file', keyFile] : []),
    ];
  }
  if (name === 'gateway') {
    return ['--bind', GATEWAY_BIND, '--web-root', ternWebRootPath(resourcePath) || ''];
  }
  return ['--port', String(opts.port)];
}

export async function runServiceCommand(
  name: LocalServiceName,
  action: 'install' | 'uninstall' | 'start' | 'stop',
  { port, run = runTern }: { port?: number; run?: Runner } = {}
): Promise<{ ok: boolean; error: string | null }> {
  const args = [
    ...base(),
    name,
    action,
    ...(action === 'install' ? installArgs(name, { port }) : []),
  ];
  try {
    const result = await run(args);
    return result.ok ? { ok: true, error: null } : { ok: false, error: errorMessage(result) };
  } catch (err) {
    return { ok: false, error: (err as Error).message };
  }
}

export interface Pairing {
  ok: boolean;
  code?: string;
  url?: string | null;
  error?: string;
}

/** `tern gateway pair`: a one-time code for a browser, valid for ten minutes. */
export async function pairBrowser(run: Runner = runTern): Promise<Pairing> {
  try {
    const result = await run([...base(), 'gateway', 'pair']);
    if (!result.ok) return { ok: false, error: errorMessage(result) };
    const { code, url } = JSON.parse(result.stdout);
    return { ok: true, code, url };
  } catch (err) {
    return { ok: false, error: (err as Error).message };
  }
}

export function servicePill(service: LocalService | null): { kind: StatusKind; text: string } {
  if (!service || !service.installed) return { kind: 'off', text: localized('Off') };
  return service.running
    ? { kind: 'ok', text: localized('Running') }
    : { kind: 'warn', text: localized('Not running') };
}

/** How often an open pane asks tern again, so a service started or stopped elsewhere shows. */
export const STATUS_POLL_MS = 10000;
