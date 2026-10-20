import fs from 'fs';
import path from 'path';

// <config-dir>/mcp.json, the policy tern-mcp reads on every tool call (docs/mcp.md, "Policy").
// Validation follows tern-mcp's: a file it would refuse is shown as an error here and never
// overwritten, so the pane can't silently widen or reset access.

export const POLICY_FILE = 'mcp.json';
export const AUDIT_FILE = 'mcp-audit.log';
export const TOKEN_FILE = 'mcp-token';
export const DEFAULT_HTTP_PORT = 2587;
export const ACCESS_LEVELS = ['read-only', 'read-write', 'read-write-send'] as const;

export interface AccountPolicy {
  enabled: boolean;
  excludedFolderIds: string[];
}

export interface McpPolicy {
  /** False refuses every tool call. Absent means on, as for tern-mcp, so it is only written
   * once the user flips the switch. */
  enabled?: boolean;
  accessLevel: string;
  /** Empty means every account. */
  enabledAccounts: { [accountId: string]: AccountPolicy };
  http: { enabled: boolean; port: number };
}

export function defaultPolicy(): McpPolicy {
  return {
    accessLevel: 'read-only',
    enabledAccounts: {},
    http: { enabled: false, port: DEFAULT_HTTP_PORT },
  };
}

const isObject = (v: unknown): v is { [key: string]: unknown } =>
  !!v && typeof v === 'object' && !Array.isArray(v);

/** A parsed mcp.json as tern-mcp's serde types take it, or the reason it would refuse it. */
export function parsePolicy(raw: unknown): McpPolicy {
  if (!isObject(raw)) throw new Error('not a JSON object');
  const policy = defaultPolicy();
  if (raw.enabled !== undefined) {
    if (typeof raw.enabled !== 'boolean') throw new Error('enabled is not true or false');
    policy.enabled = raw.enabled;
  }
  if (raw.accessLevel !== undefined && raw.accessLevel !== null) {
    if (typeof raw.accessLevel !== 'string') throw new Error('accessLevel is not a string');
    policy.accessLevel = raw.accessLevel;
  }
  if (raw.enabledAccounts !== undefined) {
    if (!isObject(raw.enabledAccounts)) throw new Error('enabledAccounts is not an object');
    for (const [id, a] of Object.entries(raw.enabledAccounts)) {
      if (!isObject(a)) throw new Error(`enabledAccounts.${id} is not an object`);
      if (a.enabled !== undefined && typeof a.enabled !== 'boolean') {
        throw new Error(`enabledAccounts.${id}.enabled is not true or false`);
      }
      const excluded = a.excludedFolderIds;
      if (
        excluded !== undefined &&
        !(Array.isArray(excluded) && excluded.every((f) => typeof f === 'string'))
      ) {
        throw new Error(`enabledAccounts.${id}.excludedFolderIds is not a list of ids`);
      }
      policy.enabledAccounts[id] = {
        enabled: a.enabled === true,
        excludedFolderIds: (excluded as string[]) || [],
      };
    }
  }
  if (raw.http !== undefined) {
    const http = raw.http;
    if (!isObject(http)) throw new Error('http is not an object');
    if (http.enabled !== undefined && typeof http.enabled !== 'boolean') {
      throw new Error('http.enabled is not true or false');
    }
    if (http.port !== undefined && !isValidPort(http.port)) {
      throw new Error('http.port is not a port number');
    }
    policy.http = {
      enabled: http.enabled === true,
      port: (http.port as number) ?? DEFAULT_HTTP_PORT,
    };
  }
  return policy;
}

export function isValidPort(port: unknown): port is number {
  return typeof port === 'number' && Number.isInteger(port) && port >= 0 && port <= 65535;
}

export interface PolicyRead {
  policy: McpPolicy;
  exists: boolean;
  error: string | null;
  /** The file the error is about. */
  errorFile?: string;
}

export function policyPath(configDir: string) {
  return path.join(configDir, POLICY_FILE);
}

function readRaw(configDir: string): { exists: boolean; raw?: unknown; error?: string } {
  let text: string;
  try {
    text = fs.readFileSync(policyPath(configDir), 'utf8');
  } catch (err) {
    if (err.code === 'ENOENT') return { exists: false };
    return { exists: true, error: err.message };
  }
  try {
    return { exists: true, raw: JSON.parse(text) };
  } catch (err) {
    return { exists: true, error: err.message };
  }
}

export function readPolicy(configDir: string): PolicyRead {
  const { exists, raw, error } = readRaw(configDir);
  if (!exists) return { policy: defaultPolicy(), exists, error: null };
  if (error) return { policy: defaultPolicy(), exists, error };
  try {
    return { policy: parsePolicy(raw), exists, error: null };
  } catch (err) {
    return { policy: defaultPolicy(), exists, error: err.message };
  }
}

/** The policy tern-mcp applies: mcp.json, else `core.mcp.*` in config.json. Without
 * mcp.json, a `core.mcp` tern-mcp refuses is reported as an error so the pane doesn't offer
 * to replace it with a wider default. */
export function readEffectivePolicy(configDir: string, coreMcp: unknown): PolicyRead {
  const read = readPolicy(configDir);
  if (read.error) return { ...read, errorFile: policyPath(configDir) };
  if (read.exists || !isObject(coreMcp)) return read;
  try {
    return { ...read, policy: policyFromCoreMcp(coreMcp) };
  } catch (err) {
    return {
      ...read,
      error: `core.mcp: ${err.message}`,
      errorFile: path.join(configDir, 'config.json'),
    };
  }
}

/** Writes the policy through a temporary file and a rename, keeping keys tern-mcp may add
 * that this pane doesn't know, at the top level and inside `http` and each account.
 * Refuses to replace a file that doesn't parse. */
export function writePolicy(configDir: string, policy: McpPolicy) {
  const { exists, raw, error } = readRaw(configDir);
  if (exists && (error || !isObject(raw))) {
    throw new Error(`${policyPath(configDir)} is not valid; fix or delete it first.`);
  }
  const old = (exists ? raw : {}) as { [key: string]: any };
  const oldAccounts = isObject(old.enabledAccounts) ? old.enabledAccounts : {};
  const enabledAccounts: { [id: string]: object } = {};
  for (const [id, a] of Object.entries(policy.enabledAccounts)) {
    enabledAccounts[id] = { ...(isObject(oldAccounts[id]) ? oldAccounts[id] : {}), ...a };
  }
  const merged = {
    ...old,
    ...policy,
    enabledAccounts,
    http: { ...(isObject(old.http) ? old.http : {}), ...policy.http },
  };
  const target = policyPath(configDir);
  const tmp = `${target}.${process.pid}.${Date.now()}.tmp`;
  fs.mkdirSync(configDir, { recursive: true });
  fs.writeFileSync(tmp, JSON.stringify(merged, null, 2) + '\n', { mode: 0o600 });
  fs.renameSync(tmp, target);
}

/** The policy the desktop's old MCP plugin kept in config.json under `core.mcp`, with the
 * null-tolerant reading tern-mcp applies to it. Throws on a shape tern-mcp would refuse.
 * HTTP stays off: the plugin's server and its token are gone, and `tern-mcp http` has its
 * own token. */
export function policyFromCoreMcp(old: { [key: string]: any }): McpPolicy {
  const policy = defaultPolicy();
  if (typeof old.accessLevel === 'string') policy.accessLevel = old.accessLevel;
  if (old.enabledAccounts !== undefined && old.enabledAccounts !== null) {
    if (!isObject(old.enabledAccounts)) throw new Error('enabledAccounts is not an object');
    for (const [id, a] of Object.entries(old.enabledAccounts)) {
      if (a === null) {
        policy.enabledAccounts[id] = { enabled: false, excludedFolderIds: [] };
        continue;
      }
      if (!isObject(a)) throw new Error(`enabledAccounts.${id} is not an object`);
      const enabled = a.enabled ?? false;
      const excluded = a.excludedFolderIds ?? [];
      if (typeof enabled !== 'boolean') throw new Error(`${id}.enabled is not a boolean`);
      if (!Array.isArray(excluded) || !excluded.every((f) => typeof f === 'string')) {
        throw new Error(`${id}.excludedFolderIds is not a list of ids`);
      }
      policy.enabledAccounts[id] = { enabled, excludedFolderIds: excluded };
    }
  }
  if (isValidPort(old.port) && old.port > 0) policy.http.port = old.port;
  return policy;
}

/** Moves `core.mcp.*` into mcp.json once. An existing mcp.json wins, as it does for
 * tern-mcp; either way the old keys are removed so they can't come back. A shape tern-mcp
 * refuses is left in place, so tern-mcp keeps refusing it rather than widening access. */
export function migrateCoreMcpConfig(configDir: string): 'migrated' | 'dropped' | 'none' {
  const old = AppEnv.config.get('core.mcp');
  if (!isObject(old)) return 'none';
  let result: 'migrated' | 'dropped' = 'dropped';
  if (!readRaw(configDir).exists) {
    let policy: McpPolicy;
    try {
      policy = policyFromCoreMcp(old);
    } catch (err) {
      console.warn(`[MCP] Not moving core.mcp to ${POLICY_FILE}: ${err.message}`);
      return 'none';
    }
    writePolicy(configDir, policy);
    result = 'migrated';
  }
  AppEnv.config.unset('core.mcp');
  return result;
}

export interface AuditEntry {
  timestamp: number;
  client?: string;
  toolName: string;
  accountId?: string | null;
  outcome?: string;
  params?: string;
  resultSummary?: string;
  durationMs?: number;
}

/** The newest `limit` entries of mcp-audit.log, newest first. Reads only the end of the
 * file, which tern-mcp rotates at 4 MiB. */
export function readAuditLog(configDir: string, limit = 50, maxBytes = 256 * 1024) {
  const file = path.join(configDir, AUDIT_FILE);
  let fd: number;
  try {
    fd = fs.openSync(file, 'r');
  } catch (err) {
    return [];
  }
  try {
    const size = fs.fstatSync(fd).size;
    const start = Math.max(0, size - maxBytes);
    const buffer = Buffer.alloc(size - start);
    fs.readSync(fd, buffer, 0, buffer.length, start);
    const lines = buffer.toString('utf8').split('\n');
    // A read that starts mid-file starts mid-line.
    if (start > 0) lines.shift();
    const entries: AuditEntry[] = [];
    for (const line of lines.reverse()) {
      if (entries.length >= limit) break;
      if (!line.trim()) continue;
      try {
        const entry = JSON.parse(line);
        if (isObject(entry) && typeof entry.toolName === 'string') entries.push(entry as any);
      } catch (err) {
        // a line being written, or a damaged one
      }
    }
    return entries;
  } finally {
    fs.closeSync(fd);
  }
}
