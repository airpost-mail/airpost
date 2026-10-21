// SPDX-License-Identifier: GPL-3.0-only
import fs from 'fs';
import path from 'path';
import { EventEmitter } from 'events';
import type { McpHttpStatus } from '../../platform/services';
import { SupervisedChild } from './supervised-child';

// Preferences > MCP > HTTP: while the app runs and `http.enabled` is true in mcp.json, main
// runs `tern-mcp http` on `http.port` (apps/mcp/src/http.rs). mcp.json is polled, so a change
// made by the pane, `tern` or an editor applies within a couple of seconds; the pane also asks
// for a reload after it saves.

const POLICY_FILE = 'mcp.json';
const TOKEN_FILE = 'mcp-token';
export const DEFAULT_MCP_HTTP_PORT = 2587;
const POLL_MS = 2000;

export interface HttpPolicy {
  enabled: boolean;
  port: number;
  error: string | null;
}

/** mcp.json's `http`, read as tern-mcp reads it; a file that doesn't parse turns HTTP off. */
export function readHttpPolicy(configDir: string): HttpPolicy {
  const off = (error: string | null = null) => ({
    enabled: false,
    port: DEFAULT_MCP_HTTP_PORT,
    error,
  });
  let raw: any;
  try {
    raw = JSON.parse(fs.readFileSync(path.join(configDir, POLICY_FILE), 'utf8'));
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return off();
    return off((err as Error).message);
  }
  const http = raw && typeof raw === 'object' ? raw.http : undefined;
  if (http === undefined) return off();
  if (!http || typeof http !== 'object') return off('http is not an object');
  const port = http.port ?? DEFAULT_MCP_HTTP_PORT;
  if (!Number.isInteger(port) || port < 0 || port > 65535) {
    return off('http.port is not a port number');
  }
  return { enabled: http.enabled === true, port, error: null };
}

// `[tern-mcp] listening on http://127.0.0.1:2587/mcp`
export function mcpReadyLine(line: string) {
  const match = /listening on (http:\/\/\S+)/.exec(line);
  return match ? { url: match[1] } : null;
}

export class McpHttpService extends EventEmitter {
  readonly child = new SupervisedChild({ name: 'tern-mcp http', ready: mcpReadyLine });
  private policy: HttpPolicy = { enabled: false, port: DEFAULT_MCP_HTTP_PORT, error: null };
  private watching = false;
  private onPolicyFile = () => {
    this.reload();
  };

  constructor(
    private configDir: string,
    private binary: () => string | null
  ) {
    super();
    this.child.on('change', () => this.emit('change', this.status()));
  }

  private policyFile() {
    return path.join(this.configDir, POLICY_FILE);
  }

  start() {
    if (!this.watching) {
      this.watching = true;
      fs.watchFile(this.policyFile(), { interval: POLL_MS, persistent: false }, this.onPolicyFile);
    }
    return this.reload();
  }

  async reload() {
    this.policy = readHttpPolicy(this.configDir);
    const binary = this.binary();
    const { enabled, port } = this.policy;
    await this.child.setSpec(
      enabled && binary
        ? {
            command: binary,
            args: ['--config-dir', this.configDir, 'http', '--port', String(port)],
          }
        : null
    );
    this.emit('change', this.status());
    return this.status();
  }

  status(): McpHttpStatus {
    return {
      service: 'mcp-http',
      ...this.child.status(),
      enabled: this.policy.enabled,
      port: this.policy.port,
      available: !!this.binary(),
      tokenFile: path.join(this.configDir, TOKEN_FILE),
      policyError: this.policy.error,
    };
  }

  stop() {
    if (this.watching) fs.unwatchFile(this.policyFile(), this.onPolicyFile);
    this.watching = false;
    return this.child.stop();
  }
}
