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
  // Reads; an engine has them when `hello.features` lists their Feature.
  CountsGet: 'counts.get',
  TasksList: 'tasks.list',
  MetadataList: 'metadata.list',
  AccountList: 'account.list',
  EngineLogsTail: 'engine.logs.tail',
  // The engine-owned settings (Feature.Settings).
  SettingsGet: 'settings.get',
  SettingsSet: 'settings.set',
  // Mail actions (Feature.Snooze, Feature.Unsubscribe).
  ThreadsSnooze: 'threads.snooze',
  ThreadsUnsnooze: 'threads.unsnooze',
  MessagesUnsubscribe: 'messages.unsubscribe',
  // Provider calls (Feature.Integrations), answered out of order; the params carry the
  // provider config, key included (docs/integrations-plan.md section 4).
  IntegrationsTranslate: 'integrations.translate',
  IntegrationsDetect: 'integrations.detect',
  IntegrationsLanguages: 'integrations.languages',
  IntegrationsGrammar: 'integrations.grammar',
  IntegrationsEnrich: 'integrations.enrich',
  IntegrationsTest: 'integrations.test',
  // Model calls (Feature.Ai), answered out of order (docs/ai-plan.md section 3). Until the
  // engine keeps the AI settings, the params carry the provider config with its key and the
  // switches (`settings`).
  AiModels: 'ai.models',
  AiTest: 'ai.test',
  AiSummarize: 'ai.summarize',
  AiSuggestReplies: 'ai.suggestReplies',
  AiDraftReply: 'ai.draftReply',
  AiCancel: 'ai.cancel',
  AiUsage: 'ai.usage',
  // `serve` mode's stdin only: the account JSON carries credentials.
  AccountAdd: 'account.add',
  AccountRemove: 'account.remove',
  AccountTest: 'account.test',
  AccountReset: 'account.reset',
  EngineResetDatabase: 'engine.resetDatabase',
  EngineSetLogLevel: 'engine.setLogLevel',
  EngineShutdown: 'engine.shutdown',
  // `serve` mode's stdin, debug builds only.
  TestCrash: 'test.crash',
  TestPanic: 'test.panic',
  // Notifications from the engine.
  Delta: 'delta',
  Disconnect: 'disconnect',
  AccountState: 'account.state',
  MailNew: 'mail.new',
  SettingsChanged: 'settings.changed',
  // The whole new `capabilities` (Feature.Capabilities), to every subscriber.
  CapabilitiesChanged: 'capabilities.changed',
  // `{streamId, text}`, only to the connection that sent `streamId`, ahead of the reply.
  AiDelta: 'ai.delta',
} as const;

// What `hello.features` may list (ms-ipc `feature`). Clients gate on these, never on versions;
// an older engine without `features` has none.
export const Feature = {
  Counts: 'counts',
  TasksList: 'tasks-list',
  MetadataList: 'metadata-list',
  MessageHidden: 'message-hidden',
  MailNew: 'mail-new',
  Settings: 'settings',
  // The engine acts on due send-later, thread-snooze and send-reminders metadata itself; only
  // send-later rows marked `clientFinalize` still need a window (docs/desktop-client-plan.md 3.3).
  ExpirationExecutor: 'expiration-executor',
  // `threads.snooze` and `threads.unsnooze`.
  Snooze: 'snooze',
  // `messages.unsubscribe`: the engine makes the one-click POST or sends the mailto message.
  Unsubscribe: 'unsubscribe',
  Integrations: 'integrations',
  // `capabilities` in hello and engine.status, and capabilities.changed.
  Capabilities: 'capabilities',
  // The `ai.*` methods but triage, `ai.delta`, and `capabilities.ai`.
  Ai: 'ai',
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
  // A relayed connection asked for a method its scopes don't allow.
  ForbiddenRelayed: -32008,
  TooManyConnections: -32009,
  // A provider call failed; `data` is the IntegrationError `{key, message, retryable, ...}`.
  IntegrationFailed: -32010,
  // A model call failed, or the account or feature is off; `data` is the AiError
  // `{key, message, retryable, status?, retryAfter?}`.
  AiFailed: -32011,
  // The workers for this kind of call are busy and their queue is full; try again later.
  EngineBusy: -32012,
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
  features?: string[];
  capabilities?: EngineCapabilities;
}

// One entry of `capabilities` (ms-ipc `Capability`). `reason` is one of CapabilityReason, or one
// a newer engine adds.
export interface EngineCapability {
  available: boolean;
  provider?: string;
  // 'request': the client sends the provider's key with each call, so it checks for one.
  credentials?: 'request' | string;
  reason?: string;
  // AI features: the model runs on this computer or network.
  local?: boolean;
}

// What the engine can do for its clients now. `integrations` by kind (translation, grammar,
// enrichment), `server` by feature (readReceipts, linkTracking, threadSharing, activitySharing,
// metadataSync), `ai` by feature (summary, reply, triage); newer engines add groups.
export interface EngineCapabilities {
  integrations?: { [kind: string]: EngineCapability };
  server?: { [feature: string]: EngineCapability };
  ai?: { [feature: string]: EngineCapability };
  [group: string]: { [name: string]: EngineCapability } | undefined;
}

export const CapabilityReason = {
  NotSetUp: 'not-set-up',
  MissingUrl: 'missing-url',
  NeedsTernServer: 'needs-tern-server',
} as const;

export interface CategoryCounts {
  unread: number;
  total: number;
}

// `accounts` null (or absent) means every account the engine holds.
export interface CountsGetParams {
  accounts?: string[] | null;
}

// Keyed by category id. The same rows arrive as ThreadCounts deltas after it.
export interface CountsGetResult {
  counts: { [categoryId: string]: CategoryCounts };
}

// A ThreadCounts delta's model: `persist` when a count changes or a category is added,
// `unpersist` (no counts) when the category is removed.
export interface ThreadCountsRow {
  id: string;
  categoryId: string;
  unread?: number;
  total?: number;
}

export const THREAD_COUNTS_CLASS = 'ThreadCounts';

// Once per message that lands unread in the inbox after the first headers sync.
export interface MailNewParams {
  accountId: string;
  threadId: string;
  messageId: string;
}

// `settings.get` params; without `keys`, every stored value.
export interface SettingsGetParams {
  keys?: string[];
}

// `settings.get`'s result, `settings.set`'s params and `settings.changed`'s params. Keys are
// config key paths; null clears a key (set) or says it was cleared (changed), and a key
// without a stored value is absent.
export interface SettingsValues {
  values: { [keyPath: string]: unknown };
  // settings.get only: the store's id, new whenever the engine starts a store from nothing
  // (no settings.json, or a corrupt one set aside).
  storeId?: string;
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
  features?: string[];
  capabilities?: EngineCapabilities;
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

// `account.test`'s result, `--mode test`'s object: `account` (with any rotated refresh token)
// on success, `error` (an ErrorCodeToTypeMap name) and `log` on failure.
export interface AccountTestResult {
  account: Record<string, any> | null;
  error: string | null;
  error_service?: string;
  error_advice?: string;
  error_offline?: boolean;
  log: string;
}

export interface ResetDatabaseResult {
  removed: string[];
}

export interface SetLogLevelResult {
  restarted: string[];
}

// The most `engine.logs.tail` returns (ms-ipc LOGS_TAIL_MAX_BYTES).
export const LOGS_TAIL_MAX_BYTES = 1024 * 1024;

// Without `accountId`, the engine's own log.
export interface LogsTailParams {
  accountId?: string;
  bytes?: number;
}

export interface LogsTailResult {
  text: string;
  truncated: boolean;
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

// `messages.unsubscribe` (ms-ipc `MessagesUnsubscribeParams`/`Result`). Without `method` the
// engine picks one-click, else mailto, else web; a web link comes back for the client to open.
export type UnsubscribeMethod = 'one-click' | 'mailto' | 'web';

export interface MessagesUnsubscribeParams {
  accountId: string;
  messageId: string;
  method?: UnsubscribeMethod;
}

export interface MessagesUnsubscribeResult {
  method: UnsubscribeMethod;
  ok: boolean;
  url?: string;
  error?: string;
  taskId?: string;
}

// `ai.*` results (crates/ms-ai, docs/ai-plan.md section 3).

export interface AiModelInfo {
  id: string;
  name: string;
  contextTokens?: number;
}

export interface AiTestReport {
  host: string;
  model: string;
  local: boolean;
  latencyMs: number;
}

export interface AiTokenUsage {
  inputTokens: number;
  outputTokens: number;
  estimated?: boolean;
}

// `ai.summarize` and `ai.draftReply`; `cached` only on summaries.
export interface AiCompletion {
  text: string;
  model: string;
  cached?: boolean;
  usage?: AiTokenUsage;
}

export interface AiSuggestion {
  label: string;
  text: string;
}

export interface AiDeltaParams {
  streamId: string;
  text: string;
}
