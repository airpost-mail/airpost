// SPDX-License-Identifier: GPL-3.0-only

// The programs main runs or installs next to the app (main/services/, handlers in
// main/ipc/services.ts): MCP over HTTP (`tern-mcp http`, Preferences > MCP), web access
// (`tern-gateway serve`) and the engine's user unit (`tern engine install`), both in
// Preferences > General.
export const ServicesChannel = {
  // invoke -> McpHttpStatus
  McpHttpStatus: 'services:mcp-http-status',
  // invoke -> McpHttpStatus. Rereads mcp.json now rather than at the next poll.
  McpHttpReload: 'services:mcp-http-reload',
  // invoke -> GatewayStatus
  GatewayStatus: 'services:gateway-status',
  // invoke -> GatewayPairing. Runs `tern-gateway pair`.
  GatewayPair: 'services:gateway-pair',
  // invoke -> EngineUnitStatus
  EngineUnitStatus: 'services:engine-unit-status',
  // invoke(install: boolean) -> EngineUnitResult
  EngineUnitSet: 'services:engine-unit-set',
  // send(McpHttpStatus | GatewayStatus) to every app window when either changes; the
  // payload's `service` says which.
  Changed: 'services:changed',
} as const;

export type ServiceState = 'stopped' | 'starting' | 'running' | 'error';

export interface ServiceStatus {
  state: ServiceState;
  url: string | null;
  /** The child's last line on stderr when it exited, or why it couldn't start. */
  error: string | null;
  pid: number | null;
}

export interface McpHttpStatus extends ServiceStatus {
  service: 'mcp-http';
  /** `http.enabled` in mcp.json. */
  enabled: boolean;
  port: number;
  /** This build has tern-mcp. */
  available: boolean;
  tokenFile: string;
  /** mcp.json doesn't parse; HTTP stays off until it does. */
  policyError: string | null;
}

export interface GatewayStatus extends ServiceStatus {
  service: 'gateway';
  /** `core.services.webAccess`. */
  enabled: boolean;
  /** This build has tern-gateway and the web client. */
  available: boolean;
  missing: ('tern-gateway' | 'web')[];
}

export interface GatewayPairing {
  ok: boolean;
  code?: string;
  url?: string | null;
  expiresAt?: number;
  error?: string;
}

export interface EngineUnitStatus {
  /** This build has tern and launchd or systemd to install with. */
  available: boolean;
  installed: boolean;
  /** The unit's engine is running now. */
  running: boolean;
  /** This app is using the unit's engine rather than its own. */
  joined: boolean;
  unit: string | null;
}

export interface EngineUnitResult {
  ok: boolean;
  error?: string;
  status: EngineUnitStatus;
}
