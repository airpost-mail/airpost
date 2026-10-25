// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The Rust engine's socket protocol, v1: JSON-RPC 2.0 over newline-delimited JSON on a Unix
socket. The method, notification and model types and the constants are generated from
crates/tern-ipc (packages/client, `@tern/client`) and re-exported here with the package's
JSON-RPC envelope and session, some under the names the desktop has used; what stays here are
the desktop's socket helpers.
*/
import crypto from 'crypto';
import os from 'os';
import path from 'path';
import {
  Method,
  type Capabilities,
  type Capability,
  type ContactDetails,
  type CredentialsImportParams as AnyCredentialsImportParams,
  type QueryResult as AnyQueryResult,
} from '@tern/client';

export * from '@tern/client';
export type {
  Capability as EngineCapability,
  EventConflict as EventConflictJSON,
  EventGuest as EventGuestJSON,
  EventWrite as EventWriteJSON,
  SyncErrorJson as SyncErrorJSON,
  ThreadCountsJson as ThreadCountsRow,
} from '@tern/client';

// The generated types narrowed to what the desktop reads and builds.

// Groups a newer engine adds are kept as they came (the plugin API's `EngineCapabilities`).
export interface EngineCapabilities extends Capabilities {
  [group: string]: { [name: string]: Capability } | undefined;
}

// `query`'s rows, which the desktop inflates into models by key.
export interface QueryResult extends Omit<AnyQueryResult, 'rows'> {
  rows: Record<string, any>[];
}

// The desktop sends both fields, each account with its secrets in `settings`.
export interface CredentialsImportParams extends Required<AnyCredentialsImportParams> {
  accounts: Array<Record<string, any>>;
}

// `data` is the contact editor's shape (features/contacts ContactBase); tern-ipc leaves it
// untyped.
export interface ContactDetailsJSON extends Omit<ContactDetails, 'data'> {
  data: any;
}

// macOS allows 104 bytes in sun_path including the NUL; Linux 108. The smaller one wins.
export const MAX_SOCKET_PATH_BYTES = 103;

// The JSON-RPC envelope (`RpcResponse`, `RpcErrorObject`, ...), the RpcError and
// EngineUnavailableError a call rejects with, and the session come from `@tern/client`.
export type { RpcId as RequestId } from '@tern/client';

// `serve`'s exit code when another engine is listening on its socket, typically the previous
// one still shutting down (tern_core EXIT_SOCKET_IN_USE).
export const EXIT_SOCKET_IN_USE = 3;

/*
Where `serve` listens: `<configDir>/run/engine.sock`, or, when that is too long for sun_path
(configDir under /var/folders, a long profile path), a short name in a per-user directory under
the temp dir. Returns null if neither fits.
*/
export function serveSocketPath(
  configDirPath: string
): { socketPath: string; ownDir: string | null } | null {
  const preferred = path.join(configDirPath, 'run', 'engine.sock');
  if (Buffer.byteLength(preferred) <= MAX_SOCKET_PATH_BYTES) {
    return { socketPath: preferred, ownDir: null };
  }
  const uid = typeof process.getuid === 'function' ? process.getuid() : 0;
  const ownDir = path.join(os.tmpdir(), `tern-${uid}`);
  // Hashed as before the per-account engines went, so an existing fallback path is kept.
  const hash = crypto
    .createHash('sha1')
    .update(`${configDirPath}engine`)
    .digest('hex')
    .slice(0, 12);
  const socketPath = path.join(ownDir, `${hash}.sock`);
  if (Buffer.byteLength(socketPath) > MAX_SOCKET_PATH_BYTES) return null;
  return { socketPath, ownDir };
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

// The `code`, `message` and `data` of a rejected request, whatever it was rejected with: an
// RpcError, EngineUnavailableError or EngineRequestError, a plain Error, or anything else.
export function rejectionFields(err: unknown): {
  code?: unknown;
  message?: unknown;
  data?: unknown;
} {
  if (typeof err !== 'object' || err === null) return {};
  return {
    code: 'code' in err ? err.code : undefined,
    message: 'message' in err ? err.message : undefined,
    data: 'data' in err ? err.data : undefined,
  };
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
