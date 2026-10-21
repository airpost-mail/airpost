// SPDX-License-Identifier: GPL-3.0-only
import { EventEmitter } from 'events';
import { execFile } from 'child_process';
import type { GatewayPairing, GatewayStatus } from '../../platform/services';
import { SupervisedChild } from './supervised-child';

// Preferences > General > Web access: while the app runs and `core.services.webAccess` is on,
// main runs `tern-gateway serve` (apps/gateway) on 127.0.0.1, serving the bundled web client
// and relaying it to this profile's engine. Only this computer can reach it; README.md in
// apps/gateway covers putting a TLS proxy in front for other machines.

export const GATEWAY_BIND = '127.0.0.1:5180';
const PAIR_TIMEOUT_MS = 15000;

// `INFO tern_gateway::cli: tern-gateway serving listen=127.0.0.1:5180 public_url=http://…`
export function gatewayReadyLine(line: string) {
  if (!line.includes('tern-gateway serving')) return null;
  const url = /public_url=(\S+)/.exec(line);
  return { url: url ? url[1].replace(/^"|"$/g, '') : `http://${GATEWAY_BIND}` };
}

export class GatewayService extends EventEmitter {
  readonly child = new SupervisedChild({ name: 'tern-gateway', ready: gatewayReadyLine });
  private enabled = false;

  constructor(
    private configDir: string,
    private binary: () => string | null,
    private webRoot: () => string | null
  ) {
    super();
    this.child.on('change', () => this.emit('change', this.status()));
  }

  async setEnabled(enabled: boolean) {
    this.enabled = enabled;
    const binary = this.binary();
    const webRoot = this.webRoot();
    await this.child.setSpec(
      enabled && binary && webRoot
        ? {
            command: binary,
            args: [
              '--config-dir',
              this.configDir,
              'serve',
              '--bind',
              GATEWAY_BIND,
              '--web-root',
              webRoot,
            ],
          }
        : null
    );
    this.emit('change', this.status());
    return this.status();
  }

  status(): GatewayStatus {
    const missing: GatewayStatus['missing'] = [];
    if (!this.binary()) missing.push('tern-gateway');
    if (!this.webRoot()) missing.push('web');
    return {
      service: 'gateway',
      ...this.child.status(),
      enabled: this.enabled,
      available: missing.length === 0,
      missing,
    };
  }

  /** A one-time code for a browser (`tern-gateway pair`), valid for ten minutes. */
  pair(): Promise<GatewayPairing> {
    const binary = this.binary();
    if (!binary) return Promise.resolve({ ok: false, error: 'tern-gateway is not in this build' });
    return new Promise((resolve) => {
      execFile(
        binary,
        ['--config-dir', this.configDir, 'pair', '--json'],
        { timeout: PAIR_TIMEOUT_MS },
        (err, stdout, stderr) => {
          if (err) {
            resolve({ ok: false, error: String(stderr || '').trim() || err.message });
            return;
          }
          try {
            const { code, url, expiresAt } = JSON.parse(String(stdout));
            resolve({ ok: true, code, url, expiresAt });
          } catch (parseErr) {
            resolve({ ok: false, error: `tern-gateway pair printed ${String(stdout).trim()}` });
          }
        }
      );
    });
  }

  stop() {
    return this.child.stop();
  }
}
