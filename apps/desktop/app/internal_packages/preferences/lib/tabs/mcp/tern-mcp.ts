import fs from 'fs';
import path from 'path';
import { execFile } from 'child_process';

// The bundled tern-mcp binary (apps/mcp), shipped next to mailsync-rs in app.asar.unpacked.
// Installing into an MCP client, the status and the stale-entry check all run tern-mcp, so
// the desktop never writes a client's config itself.

export type McpClientId = 'claude-desktop' | 'claude-code' | 'codex';

export const MCP_CLIENTS: { id: McpClientId; name: string }[] = [
  { id: 'claude-desktop', name: 'Claude Desktop' },
  { id: 'claude-code', name: 'Claude Code' },
  { id: 'codex', name: 'Codex' },
];

const STALE_ENTRY_CHECKED = 'core.ternMcp.staleBridgeEntryChecked';

export interface RunResult {
  ok: boolean;
  stdout: string;
  stderr: string;
}

/** `TERN_MCP_BIN` (a source checkout), else the binary in this build. */
export function ternMcpBinary(): string | null {
  const fromEnv = process.env.TERN_MCP_BIN;
  if (fromEnv) return fs.existsSync(fromEnv) ? fromEnv : null;
  const name = process.platform === 'win32' ? 'tern-mcp.exe' : 'tern-mcp';
  const bundled = path
    .join(AppEnv.getLoadSettings().resourcePath, name)
    .replace('app.asar', 'app.asar.unpacked');
  return fs.existsSync(bundled) ? bundled : null;
}

export function runTernMcp(args: string[], timeout = 60000): Promise<RunResult> {
  const writesClientConfig = args[0] === 'install' || args[0] === 'uninstall';
  if (AppEnv.inSpecMode() && writesClientConfig && !args.includes('--config-file')) {
    return Promise.reject(new Error('specs must give tern-mcp install --config-file'));
  }
  const binary = ternMcpBinary();
  if (!binary) return Promise.reject(new Error('tern-mcp is not in this build of Tern'));
  return new Promise((resolve) => {
    execFile(binary, args, { timeout, maxBuffer: 4 * 1024 * 1024 }, (err, stdout, stderr) => {
      resolve({ ok: !err, stdout: String(stdout || ''), stderr: String(stderr || '') });
    });
  });
}

/** tern-mcp prints results on stdout and errors on stderr as `[tern-mcp] <message>`. */
export function resultMessage(result: RunResult) {
  const text = (result.ok ? result.stdout : result.stderr || result.stdout).trim();
  return text.replace(/^\[tern-mcp\] /gm, '');
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
  return runTernMcp(args);
}

export function uninstallClient(client: McpClientId, configDir: string, configFile?: string) {
  return runTernMcp(['uninstall', ...clientArgs(client, configDir, configFile)]);
}

export interface TernMcpStatus {
  engines: {
    pid: number;
    engine: string;
    accounts: { accountId: string; email: string; state: string }[];
  }[];
  problems: string[];
  policy: { error?: string; accessLevel?: string; source?: string };
}

export async function ternMcpStatus(configDir: string): Promise<TernMcpStatus> {
  const result = await runTernMcp(['--config-dir', configDir, 'status', '--json'], 20000);
  if (!result.ok) throw new Error(resultMessage(result) || 'tern-mcp status failed');
  return JSON.parse(result.stdout);
}

/**
 * Whether this copy of the app is where it will keep running. tern-mcp writes its own path
 * into the client's config, so a dev build, a mounted DMG or a translocated copy in
 * ~/Downloads would leave an entry that breaks once the app is moved.
 */
export function isStableInstall({
  devMode,
  platform,
  inApplicationsFolder,
}: {
  devMode: boolean;
  platform: string;
  inApplicationsFolder: () => boolean;
}) {
  if (devMode) return false;
  return platform !== 'darwin' || inApplicationsFolder();
}

/**
 * The desktop's old MCP plugin registered Claude Desktop's `tern` entry as a bridge script
 * that no longer ships. Once per profile, tern-mcp replaces that entry if it is pinned to
 * this profile (`install --if-stale`; tern-mcp does the detection). Called at startup of
 * the real app's main window only, never from specs: it edits Claude Desktop's own config.
 */
export async function replaceStaleBridgeEntryOnce(configDir: string) {
  if (AppEnv.inSpecMode() || !AppEnv.isMainWindow()) return;
  if (AppEnv.config.get(STALE_ENTRY_CHECKED)) return;
  // Not yet installed for good: leave the flag unset so a later launch does the check.
  const stable = isStableInstall({
    devMode: AppEnv.inDevMode(),
    platform: process.platform,
    inApplicationsFolder: () => require('@electron/remote').app.isInApplicationsFolder(),
  });
  if (!stable) return;
  if (!ternMcpBinary()) return;
  const result = await runTernMcp([
    'install',
    '--if-stale',
    ...clientArgs('claude-desktop', configDir),
  ]);
  const message = resultMessage(result);
  if (result.ok) {
    AppEnv.config.set(STALE_ENTRY_CHECKED, true);
    if (!message.startsWith('No stale')) console.log(`[MCP] ${message}`);
  } else {
    console.warn(`[MCP] Could not check Claude Desktop's tern entry: ${message}`);
  }
}
