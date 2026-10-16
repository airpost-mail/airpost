// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The Rust engine's socket protocol, v1 (docs/engine-protocol-plan.md section 1): JSON-RPC 2.0
over newline-delimited JSON on a Unix socket. Hand-written from crates/ms-ipc; generating these
from it waits until the surface grows (plan section 2).
*/
import crypto from 'crypto';
import os from 'os';
import path from 'path';

export const PROTOCOL_VERSION = 1;

// The longest line the engine sends or accepts, without its newline (ms-ipc MAX_LINE_BYTES).
export const MAX_LINE_BYTES = 64 * 1024 * 1024;

// macOS allows 104 bytes in sun_path including the NUL; Linux 108. The smaller one wins.
export const MAX_SOCKET_PATH_BYTES = 103;

export type RequestId = number | string;

export interface RpcRequest<P = unknown> {
  jsonrpc: '2.0';
  id?: RequestId;
  method: string;
  params?: P;
}

export interface RpcError {
  code: number;
  message: string;
  data?: unknown;
}

export interface RpcResponse<R = unknown> {
  jsonrpc: '2.0';
  id: RequestId | null;
  result?: R;
  error?: RpcError;
}

export interface RpcNotification<P = unknown> {
  jsonrpc: '2.0';
  method: string;
  params: P;
}

export const Method = {
  Hello: 'hello',
  Subscribe: 'subscribe',
  TaskQueue: 'task.queue',
  TaskCancel: 'task.cancel',
  SyncWake: 'sync.wake',
  BodiesFetch: 'bodies.fetch',
  CalendarSync: 'calendar.sync',
  EngineStatus: 'engine.status',
  AccountList: 'account.list',
  // `serve` mode's stdin only: the account JSON carries credentials.
  AccountAdd: 'account.add',
  AccountRemove: 'account.remove',
  EngineShutdown: 'engine.shutdown',
  // `serve` mode's stdin, debug builds only.
  TestCrash: 'test.crash',
  TestPanic: 'test.panic',
  // Notifications from the engine.
  Delta: 'delta',
  Disconnect: 'disconnect',
  AccountState: 'account.state',
} as const;

export const ErrorCode = {
  ParseError: -32700,
  InvalidRequest: -32600,
  MethodNotFound: -32601,
  InvalidParams: -32602,
  InternalError: -32603,
  // `data.supported` lists the versions the engine speaks.
  UnsupportedProtocol: -32000,
  UnknownAccount: -32001,
  // The task's local phase failed; `data` is the SyncError JSON.
  TaskRejected: -32002,
  HelloRequired: -32003,
  NotFound: -32004,
  // `data` is `{state, error}`.
  AccountStopped: -32005,
  // `account.add` of an account that is starting or running.
  AccountExists: -32006,
  // A parent-only method on the socket, or `subscribe` on stdin.
  WrongChannel: -32007,
} as const;

// `serve`'s exit code when another engine is listening on its socket, typically the previous
// one still shutting down (ms_core EXIT_SOCKET_IN_USE).
export const EXIT_SOCKET_IN_USE = 3;

export const AccountState = {
  Starting: 'starting',
  Running: 'running',
  Stopped: 'stopped',
  Removed: 'removed',
} as const;

export interface HelloParams {
  protocol: number;
  client: string;
}

export interface AccountInfo {
  accountId: string;
  email: string;
  state: string;
  error?: SyncErrorJSON;
}

// The engine's SyncError JSON, as `account.state` and the status methods carry it.
export interface SyncErrorJSON {
  what?: string;
  key: string;
  debuginfo?: string;
  retryable?: boolean;
  offline?: boolean;
}

export interface HelloResult {
  protocol: number;
  engine: string;
  pid: number;
  accounts: AccountInfo[];
}

// null means every account, or every model class.
export interface SubscribeParams {
  accounts: string[] | null;
  classes: string[] | null;
}

export interface TaskQueueParams {
  accountId: string;
  task: unknown;
}

export interface TaskCancelParams {
  accountId: string;
  taskId: string;
}

// No accountId wakes every account.
export interface SyncWakeParams {
  accountId?: string;
}

export interface BodiesFetchParams {
  accountId: string;
  ids: string[];
}

export interface CalendarSyncParams {
  accountId: string;
}

export interface CalendarSyncResult {
  started: boolean;
}

export interface AccountStatus {
  accountId: string;
  email: string;
  state: string;
  connectionError: boolean;
  error?: SyncErrorJSON;
}

export interface EngineStatusResult {
  protocol: number;
  engine: string;
  pid: number;
  uptimeSecs: number;
  accounts: AccountStatus[];
}

// Today's stdout delta object plus the account it came from.
export interface DeltaParams {
  accountId: string;
  modelClass: string;
  type: 'persist' | 'unpersist' | 'metadata-expiration' | string;
  modelJSONs: unknown[];
}

export interface AccountListResult {
  accounts: AccountStatus[];
}

// `identity` is the identity line `sync` mode reads; null for none.
export interface AccountAddParams {
  account: unknown;
  identity: unknown;
}

export interface AccountAddResult {
  accountId: string;
  state: string;
  error?: SyncErrorJSON;
}

// The account's threads still running after the engine's stop timeout. They may still write
// to its database until a blocking call returns.
export interface AccountRemoveResult {
  stillRunning: string[];
}

export interface AccountIdParams {
  accountId: string;
}

export interface AccountStateParams {
  accountId: string;
  state: 'starting' | 'running' | 'stopped' | 'removed' | string;
  error?: SyncErrorJSON;
}

export interface DisconnectParams {
  reason: string;
}

// The stdout line the engine prints once the socket is bound.
export interface ListeningLine {
  listening: string;
}

export function parseListeningLine(line: string): string | null {
  if (!line.startsWith('{"listening"')) return null;
  try {
    const json = JSON.parse(line) as ListeningLine;
    return typeof json.listening === 'string' ? json.listening : null;
  } catch (err) {
    return null;
  }
}

/*
Where the engine for `accountId` listens: `<configDir>/run/<accountId>.sock`, or, when that is
too long for sun_path (configDir under /var/folders, a long profile path), a short name in a
per-user directory under the temp dir. Returns null if neither fits.
*/
export function engineSocketPath(
  configDirPath: string,
  accountId: string
): { socketPath: string; ownDir: string | null } | null {
  const preferred = path.join(configDirPath, 'run', `${accountId}.sock`);
  if (Buffer.byteLength(preferred) <= MAX_SOCKET_PATH_BYTES) {
    return { socketPath: preferred, ownDir: null };
  }
  const uid = typeof process.getuid === 'function' ? process.getuid() : 0;
  const ownDir = path.join(os.tmpdir(), `tern-${uid}`);
  const hash = crypto
    .createHash('sha1')
    .update(configDirPath + accountId)
    .digest('hex')
    .slice(0, 12);
  const socketPath = path.join(ownDir, `${hash}.sock`);
  if (Buffer.byteLength(socketPath) > MAX_SOCKET_PATH_BYTES) return null;
  return { socketPath, ownDir };
}

// `serve` mode's socket: one per config dir, `<configDir>/run/engine.sock` when it fits.
export function serveSocketPath(configDirPath: string) {
  return engineSocketPath(configDirPath, 'engine');
}

/*
The request for one of the desktop's stdin commands, or null for one that stays on stdin
(test-crash, test-segfault, anything unknown).
*/
export function requestForCommand(
  accountId: string,
  json: { type?: string; [key: string]: any }
): { method: string; params: unknown } | null {
  switch (json.type) {
    case 'queue-task':
      return { method: Method.TaskQueue, params: { accountId, task: json.task } };
    case 'cancel-task':
      return { method: Method.TaskCancel, params: { accountId, taskId: json.taskId } };
    case 'wake-workers':
      return { method: Method.SyncWake, params: { accountId } };
    case 'need-bodies':
      return { method: Method.BodiesFetch, params: { accountId, ids: json.ids } };
    case 'sync-calendar':
      return { method: Method.CalendarSync, params: { accountId } };
    default:
      return null;
  }
}

const DELTA_PREFIX = '{"jsonrpc":"2.0","method":"delta","params":';

/*
The params of a `delta` notification line as JSON text, which is what the bridge parses, or
null if the line is something else. The engine writes exactly DELTA_PREFIX + params + '}', so
the text is sliced out rather than parsed and re-serialized.
*/
export function deltaParamsText(line: string): string | null {
  if (line.startsWith(DELTA_PREFIX) && line.endsWith('}')) {
    return line.slice(DELTA_PREFIX.length, -1);
  }
  return null;
}
