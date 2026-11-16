import fs from 'fs';
import path from 'path';
import { execFile } from 'child_process';
import { App } from '../../../../platform/app-api';

// The bundled airpost-mcp binary (apps/mcp), shipped next to airpost-engine in app.asar.unpacked.
// Installing into an MCP client, the status and the stale-entry check all run airpost-mcp, so
// the desktop never writes a client's config itself.

export type McpClientId = 'claude-desktop' | 'claude-code' | 'codex';

export const MCP_CLIENTS: { id: McpClientId; name: string }[] = [
  { id: 'claude-desktop', name: 'Claude Desktop' },
  { id: 'claude-code', name: 'Claude Code' },
  { id: 'codex', name: 'Codex' },
];

const STALE_ENTRY_CHECKED = 'core.airpostMcp.staleBridgeEntryChecked';

export interface RunResult {
  ok: boolean;
  stdout: string;
  stderr: string;
}

/** `AIRPOST_MCP_BIN` (a source checkout), else the binary in this build. */
export function airpostMcpBinary(): string | null {
  const fromEnv = process.env.AIRPOST_MCP_BIN;
  if (fromEnv) return fs.existsSync(fromEnv) ? fromEnv : null;
  const name = process.platform === 'win32' ? 'airpost-mcp.exe' : 'airpost-mcp';
  const bundled = path
    .join(AppEnv.getLoadSettings().resourcePath, name)
    .replace('app.asar', 'app.asar.unpacked');
  return fs.existsSync(bundled) ? bundled : null;
}

export function runAirpostMcp(args: string[], timeout = 60000): Promise<RunResult> {
  const writesClientConfig = args[0] === 'install' || args[0] === 'uninstall';
  if (AppEnv.inSpecMode() && writesClientConfig && !args.includes('--config-file')) {
    return Promise.reject(new Error('specs must give airpost-mcp install --config-file'));
  }
  const binary = airpostMcpBinary();
  if (!binary) return Promise.reject(new Error('airpost-mcp is not in this build of Airpost'));
  return new Promise((resolve) => {
    execFile(binary, args, { timeout, maxBuffer: 4 * 1024 * 1024 }, (err, stdout, stderr) => {
      resolve({ ok: !err, stdout: String(stdout || ''), stderr: String(stderr || '') });
    });
  });
}

/** airpost-mcp prints results on stdout and errors on stderr as `[airpost-mcp] <message>`. */
export function resultMessage(result: RunResult) {
  const text = (result.ok ? result.stdout : result.stderr || result.stdout).trim();
  return text.replace(/^\[airpost-mcp\] /gm, '');
}

function clientArgs(client: McpClientId, configDir: string, configFile?: string) {
  const args = ['--client', client, '--config-dir', configDir];
  if (configFile) args.push('--config-file', configFile);
  return args;
}

export function installClient(
  client: McpClientId,
  configDir: string,
  { force = false, configFile }: { force?: boolean; configFile?: string } = {}
) {
  const args = ['install', ...clientArgs(client, configDir, configFile)];
  if (force) args.push('--force');
  return runAirpostMcp(args);
}

export function uninstallClient(client: McpClientId, configDir: string, configFile?: string) {
  return runAirpostMcp(['uninstall', ...clientArgs(client, configDir, configFile)]);
}

export interface AirpostMcpStatus {
  engines: {
    pid: number;
    engine: string;
    accounts: { accountId: string; email: string; state: string }[];
  }[];
  problems: string[];
  policy: { error?: string; accessLevel?: string; source?: string };
}

export async function airpostMcpStatus(configDir: string): Promise<AirpostMcpStatus> {
  const result = await runAirpostMcp(['--config-dir', configDir, 'status', '--json'], 20000);
  if (!result.ok) throw new Error(resultMessage(result) || 'airpost-mcp status failed');
  return JSON.parse(result.stdout);
}

/**
 * Whether this copy of the app is where it will keep running. airpost-mcp writes its own path
 * into the client's config, so a dev build, a mounted DMG or a translocated copy in
 * ~/Downloads would leave an entry that breaks once the app is moved.
 */
export async function isStableInstall({
  devMode,
  platform,
  inApplicationsFolder,
}: {
  devMode: boolean;
  platform: string;
  inApplicationsFolder: () => Promise<boolean>;
}) {
  if (devMode) return false;
  return platform !== 'darwin' || (await inApplicationsFolder());
}

/**
 * The desktop's old MCP plugin registered Claude Desktop's `airpost` entry as a bridge script
 * that no longer ships. Once per profile, airpost-mcp replaces that entry if it is pinned to
 * this profile (`install --if-stale`; airpost-mcp does the detection). Called at startup of
 * the real app's main window only, never from specs: it edits Claude Desktop's own config.
 */
export async function replaceStaleBridgeEntryOnce(configDir: string) {
  if (AppEnv.inSpecMode() || !AppEnv.isMainWindow()) return;
  if (AppEnv.config.get(STALE_ENTRY_CHECKED)) return;
  // Not yet installed for good: leave the flag unset so a later launch does the check.
  const stable = await isStableInstall({
    devMode: AppEnv.inDevMode(),
    platform: process.platform,
    inApplicationsFolder: () => App.isInApplicationsFolder(),
  });
  if (!stable) return;
  if (!airpostMcpBinary()) return;
  const result = await runAirpostMcp([
    'install',
    '--if-stale',
    ...clientArgs('claude-desktop', configDir),
  ]);
  const message = resultMessage(result);
  if (result.ok) {
    AppEnv.config.set(STALE_ENTRY_CHECKED, true);
    if (!message.startsWith('No stale')) console.log(`[MCP] ${message}`);
  } else {
    console.warn(`[MCP] Could not check Claude Desktop's airpost entry: ${message}`);
  }
}
