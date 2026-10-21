// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The Rust engine's socket protocol, v1: JSON-RPC 2.0
over newline-delimited JSON on a Unix socket. Hand-written from crates/tern-ipc; generating these
from it waits until the surface grows.
*/
import crypto from 'crypto';
import os from 'os';
import path from 'path';

export const PROTOCOL_VERSION = 1;

// The longest line the engine sends or accepts, without its newline (tern-ipc MAX_LINE_BYTES).
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
  // A ModelQuery (Query.toJSON()) over the stored models (Feature.Query).
  Query: 'query',
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
  // The draft pipeline (Feature.DraftPipeline): drafts built as DraftFactory builds them, the send
  // checks, and sends with an undo window the engine keeps.
  DraftCreate: 'draft.create',
  DraftSend: 'draft.send',
  DraftCancelSend: 'draft.cancelSend',
  DraftValidate: 'draft.validate',
  // Calendar and contact editing (Feature.CalendarEditing, Feature.ContactEditing): occurrences
  // the engine expands, and edits it builds as ICS and vCards and queues as the usual tasks.
  EventsExpand: 'events.expand',
  EventsCreate: 'events.create',
  EventsUpdate: 'events.update',
  EventsRsvp: 'events.rsvp',
  ContactsGet: 'contacts.get',
  ContactsCreate: 'contacts.create',
  ContactsUpdate: 'contacts.update',
  ContactsImport: 'contacts.import',
  ContactsExport: 'contacts.export',
  // Mail rules (Feature.Rules): the engine applies the `mailRules` setting to arriving mail;
  // these run the rules over an account's inbox.
  RulesReprocess: 'rules.reprocess',
  RulesCancel: 'rules.cancel',
  // Provider calls (Feature.Integrations), answered out of order; the params carry the
  // provider config, key included.
  IntegrationsTranslate: 'integrations.translate',
  IntegrationsDetect: 'integrations.detect',
  IntegrationsLanguages: 'integrations.languages',
  IntegrationsGrammar: 'integrations.grammar',
  IntegrationsEnrich: 'integrations.enrich',
  IntegrationsTest: 'integrations.test',
  // Model calls (Feature.Ai), answered out of order. Until the
  // engine keeps the AI settings, the params carry the provider config with its key and the
  // switches (`settings`).
  AiModels: 'ai.models',
  AiTest: 'ai.test',
  AiSummarize: 'ai.summarize',
  AiSuggestReplies: 'ai.suggestReplies',
  AiDraftReply: 'ai.draftReply',
  AiCancel: 'ai.cancel',
  AiUsage: 'ai.usage',
  // Feature.AiTriage: start a triage job, and the opted-in accounts' triaged inbox threads.
  AiTriageRun: 'ai.triage.run',
  AiTriageList: 'ai.triage.list',
  // Account setup (Feature.AccountSetup), on stdin and the socket alike: secrets go in
  // `secrets` or `settings` and never come back.
  AccountAdd: 'account.add',
  AccountRemove: 'account.remove',
  AccountTest: 'account.test',
  AccountReset: 'account.reset',
  AccountDiscover: 'account.discover',
  AccountOAuthBegin: 'account.oauth.begin',
  AccountOAuthComplete: 'account.oauth.complete',
  AccountSetSecrets: 'account.setSecrets',
  AccountUpdate: 'account.update',
  AccountReorder: 'account.reorder',
  SecretsSet: 'secrets.set',
  SecretsDelete: 'secrets.delete',
  SecretsHas: 'secrets.has',
  ServerSet: 'server.set',
  ServerClear: 'server.clear',
  // `serve` mode's stdin only (a detached engine's socket too).
  EngineResetDatabase: 'engine.resetDatabase',
  EngineSetLogLevel: 'engine.setLogLevel',
  EngineShutdown: 'engine.shutdown',
  // stdin only: the accounts and keyset KeyManager held, moved into the engine (stage 11).
  CredentialsImport: 'credentials.import',
  // `serve` mode's stdin, debug builds only.
  TestCrash: 'test.crash',
  TestPanic: 'test.panic',
  // Notifications from the engine.
  Delta: 'delta',
  Disconnect: 'disconnect',
  AccountState: 'account.state',
  AccountChanged: 'account.changed',
  MailNew: 'mail.new',
  SettingsChanged: 'settings.changed',
  // The whole new `capabilities` (Feature.Capabilities), to every subscriber.
  CapabilitiesChanged: 'capabilities.changed',
  // `{streamId, text}`, only to the connection that sent `streamId`, ahead of the reply.
  AiDelta: 'ai.delta',
  // `{jobId, done, total}`, only to the connection that called ai.triage.run.
  AiTriageProgress: 'ai.triage.progress',
  // RulesProgressParams, only to the connection that called rules.reprocess.
  RulesProgress: 'rules.progress',
  // `{ruleId, error}`, to every subscriber: the engine disabled a failing rule (also saved in
  // `mailRules`, which settings.changed carries).
  RulesDisabled: 'rules.disabled',
} as const;

// What `hello.features` may list (tern-ipc `feature`). Clients gate on these, never on versions;
// an older engine without `features` has none.
export const Feature = {
  // The engine keeps the account registry and the secrets (accounts.json, its secret store),
  // starts registered accounts itself and restarts stopped ones;
  // `credentials.import` moves KeyManager's keyset into it.
  Accounts: 'accounts',
  // Account setup on the socket: account.discover, account.oauth.*, account.add/test with
  // `secrets`, account.setSecrets/update/reorder, account.remove {revoke, deleteData},
  // secrets.* and server.*.
  AccountSetup: 'account-setup',
  Counts: 'counts',
  // `query`: DatabaseStore reads over the socket.
  Query: 'query',
  TasksList: 'tasks-list',
  MetadataList: 'metadata-list',
  MessageHidden: 'message-hidden',
  MailNew: 'mail-new',
  Settings: 'settings',
  // The engine acts on due send-later, thread-snooze and send-reminders metadata itself; only
  // send-later rows marked `clientFinalize` still need a window.
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
  // `ai.triage.*`, `capabilities.ai.triage`, and the `core.ai.*` settings keys.
  AiTriage: 'ai-triage',
  // Stored AI providers read their keys from the secret store (`integration:ai-<id>`), and a
  // provider's key goes when settings.set removes it or points it at another server.
  AiKeys: 'ai-keys',
  // `draft.create`, `draft.send` (with the send hooks' `bodies`), `draft.cancelSend`,
  // `draft.validate`, and the `composing.quoteTexts` setting.
  DraftPipeline: 'draft-pipeline',
  // Drafts with open or link tracking on are sent tracked by the engine, and due expirations
  // are claimed on the Tern server first; clients don't transform tracked drafts.
  Tracking: 'tracking',
  // The engine runs the mail rules (`mailRules`, `mailRulesAutoSince`), with rules.reprocess,
  // rules.cancel, rules.progress and rules.disabled.
  Rules: 'rules',
  // `events.expand`, `events.create`, `events.update` and `events.rsvp`, and SyncbackEventTask's
  // `restore`.
  CalendarEditing: 'calendar-editing',
  // `contacts.get`, `contacts.create`, `contacts.update`, `contacts.import` and `contacts.export`.
  ContactEditing: 'contact-editing',
} as const;

// `draft.create` params (tern-ipc DraftCreateParams).
export interface DraftCreateParams {
  accountId?: string;
  kind: 'new' | 'reply' | 'reply-all' | 'forward' | 'mailto' | 'resurface';
  threadId?: string;
  messageId?: string;
  mailto?: string;
  // Message JSON keys over what the kind builds.
  fields?: { [key: string]: unknown };
  behavior?: 'prefer-existing' | 'prefer-existing-if-pristine';
  html?: boolean;
  tz?: string;
  // false: answered without saving it.
  save?: boolean;
}

// What a client's send hooks made of the body: the sent copy's, and each recipient's.
export interface DraftBodies {
  self?: string;
  perRecipient?: { [email: string]: string };
}

export interface DraftSendParams {
  accountId: string;
  headerMessageId: string;
  delayMs?: number;
  after?: string;
  bodies?: DraftBodies;
}

export interface DraftSendResult {
  taskId?: string;
  // Unix seconds, when the send waits out the undo window.
  scheduledAt?: number;
}

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
  // `account.add {test}` (or setSecrets, oauth.complete): the test failed; `data` is the
  // AccountTestResult.
  AccountTestFailed: -32013,
  // The OAuth exchange or profile request failed; `data` is `{key, offline?}`.
  OAuthFailed: -32014,
} as const;

// `serve`'s exit code when another engine is listening on its socket, typically the previous
// one still shutting down (tern_core EXIT_SOCKET_IN_USE).
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
  // Stopped: when the engine starts it again (ms since the epoch); absent when it won't.
  retryAt?: number;
  // Stopped because the server refused its credentials.
  authFailed?: boolean;
}

// `hello.credentials`: the engine's secret store and how many accounts it has registered.
export interface CredentialsStatus {
  store?: 'keychain' | 'secret-service' | 'file' | string;
  accounts: number;
  error?: string;
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
  // `sha256:<hex>` of the engine's executable, to compare with the bundled one.
  build?: string;
  // `serve --detached`: no parent channel; the socket takes the parent methods.
  detached?: boolean;
  credentials?: CredentialsStatus;
}

// One entry of `capabilities` (tern-ipc `Capability`). `reason` is one of CapabilityReason, or one
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
  // A Tern server is connected but doesn't offer the feature.
  NotOnServer: 'not-on-server',
  // AI triage in the background: the provider needs a key the engine doesn't hold yet.
  NeedsEngineKey: 'needs-engine-key',
  RateLimited: 'rate-limited',
  QuotaExceeded: 'quota-exceeded',
} as const;

export interface CategoryCounts {
  unread: number;
  total: number;
}

// `accounts` null (or absent) means every account the engine holds.
export interface CountsGetParams {
  accounts?: string[] | null;
}

// `query`'s answer (tern-ipc QueryResult): `rows` always (model JSON, with each `include` key),
// `ids` for idsOnly, `count` for count.
export interface QueryResult {
  rows: Record<string, any>[];
  ids?: string[];
  count?: number;
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
  // The non-secret Account JSON (fewer fields on a relayed connection).
  account?: Record<string, any>;
  retryAt?: number;
  authFailed?: boolean;
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
  // The engine's secret store took the account's secrets; it starts the account itself from
  // now on.
  stored?: boolean;
}

// `credentials.import`: accounts (with or without their secrets in `settings`) and the keyset,
// whose `<email>-imap|smtp|refresh-token` entries go to every account with that email and
// whose `integration-<provider>-key` and `tern-server-token` are stored too.
export interface CredentialsImportParams {
  accounts: Array<Record<string, any>>;
  secrets: { [name: string]: string };
}

export interface CredentialsImportResult {
  imported: string[];
  failed: Array<{ accountId: string; email: string; error: string }>;
  secrets: string[];
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

// An account's secrets, under the settings keys they replace. Write-only.
export interface AccountSecrets {
  imap_password?: string;
  smtp_password?: string;
  refresh_token?: string;
}

export interface AccountDiscoverResult {
  // {emailAddress, provider, settings}: servers, ports, usernames, security (never `none`),
  // `*_allow_insecure_ssl` and `container_folder`. An empty `imap_host`: nothing found.
  account: Record<string, any>;
  source: 'matcher' | 'autoconfig' | 'provider-table' | 'fallback';
  template?: string;
}

export interface OAuthBeginParams {
  provider: 'gmail' | 'office365' | 'outlook';
  sharedMailbox?: string;
  redirectPort?: number;
  // Reconnect: the account the sign-in renews.
  accountId?: string;
  containerFolder?: string;
}

export interface OAuthBeginResult {
  sessionId: string;
  authUrl: string;
  redirectUri: string;
  state: string;
}

export interface OAuthCompleteResult extends AccountAddResult {
  // The account's JSON without secrets.
  account: Record<string, any>;
}

export interface AccountUpdateResult {
  account: Record<string, any>;
  secretsCleared?: boolean;
  restarted?: boolean;
}

export interface ResetDatabaseResult {
  removed: string[];
}

export interface SetLogLevelResult {
  restarted: string[];
}

// The most `engine.logs.tail` returns (tern-ipc LOGS_TAIL_MAX_BYTES).
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
  // Stopped: when the engine's restart policy starts it again; absent when it won't.
  retryAt?: number;
  // Stopped because the server refused its credentials; only account.add restarts it.
  authFailed?: boolean;
}

// `account.changed`: a registered account's non-secret JSON after the engine stored it.
export interface AccountChangedParams {
  accountId: string;
  account: Record<string, any>;
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

// The `code`, `message` and `data` of a rejected request, whatever it was rejected with: an
// EngineConnectionError or EngineRequestError, a plain Error, or anything else.
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

// `messages.unsubscribe` (tern-ipc `MessagesUnsubscribeParams`/`Result`). Without `method` the
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

// `ai.*` results (crates/tern-ai).

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

export interface AiTriageRunResult {
  jobId: string;
  queued: number;
}

export interface AiTriageProgressParams {
  jobId: string;
  done: number;
  total: number;
  /** The job ended after `done` threads: cancelled, the account stopped or triage turned off. */
  stopped?: boolean;
}

export interface RulesReprocessResult {
  jobId: string;
  // The inbox threads the job goes through.
  total: number;
}

export interface RulesProgressParams {
  jobId: string;
  accountId: string;
  done: number;
  total: number;
  // The job ended after `done` threads: cancelled, or the account stopped.
  stopped?: boolean;
}

export interface AiTriageListResult {
  threadIds: string[];
  total: number;
}

// One `ai.usage` row, newest day first.
export interface AiUsageRow {
  day: string;
  provider: string;
  feature: string;
  inputTokens: number;
  outputTokens: number;
  requests: number;
}

// Calendar and contact editing (tern-ipc pim.rs). Times are epoch seconds; `tz` is the zone floating
// times, DATE values and all-day events are read in.

export interface EventsExpandParams {
  accountIds?: string[];
  calendarIds?: string[];
  eventIds?: string[];
  // An ICS that is not stored (an emailed invitation), read as `accountIds[0]` would have it.
  ics?: string;
  start: number;
  end: number;
  tz?: string;
  focus?: { now: number; recurrenceIdStart?: number };
  conflicts?: { excludeIcsuid?: string };
}

// `upcoming` and `first`: an invitation's next occurrence and its own times.
export interface EventTimes {
  start: number;
  end: number;
  isAllDay: boolean;
  recurrenceId?: number;
}

export interface EventConflictJSON {
  eventId: string;
  calendarId: string;
  icsuid: string;
  title: string;
  start: number;
  end: number;
}

// An occurrence has the calendar's EventOccurrence keys, plus `eventId`, `icsuid`, `myStatus`,
// `rrule` and `timezone`.
export interface EventsExpandResult {
  occurrences: any[];
  focused?: any | null;
  upcoming?: EventTimes | null;
  first?: EventTimes | null;
  conflicts?: EventConflictJSON[];
}

export interface EventGuestJSON {
  email: string;
  name?: string;
  role?: string;
}

export interface EventsCreateParams {
  accountId: string;
  calendarId: string;
  summary: string;
  start: number;
  end: number;
  isAllDay?: boolean;
  timezone?: string;
  description?: string;
  location?: string;
  attendees?: EventGuestJSON[];
  recurrenceRule?: string;
  transparency?: string;
  tz?: string;
  queue?: boolean;
}

// `deleted: true` removes the occurrences the scope covers; `recurrenceRule: null` removes the rule.
export interface EventPatch {
  start?: number;
  end?: number;
  isAllDay?: boolean;
  timezone?: string;
  summary?: string;
  location?: string;
  description?: string;
  attendees?: EventGuestJSON[];
  transparency?: string;
  recurrenceRule?: string | null;
  deleted?: boolean;
}

export type EventScope = 'this' | 'following' | 'all';

export interface EventsUpdateParams {
  accountId: string;
  eventId: string;
  patch: EventPatch;
  scope?: EventScope;
  // The occurrence, by the slot it fills (its recurrenceIdStart, else its start).
  occurrenceStart?: number;
  tz?: string;
  queue?: boolean;
}

export interface EventWriteJSON {
  kind: 'create' | 'update' | 'destroy';
  eventId?: string;
  calendarId?: string;
  ics?: string;
  taskId?: string;
}

export interface EventsWriteResult {
  writes: EventWriteJSON[];
  icsuid?: string;
}

export interface EventsRsvpParams {
  accountId: string;
  eventId?: string;
  ics?: string;
  messageId?: string;
  status: string;
  comment?: string;
  counter?: { start: number; end: number };
  occurrenceStart?: number;
  calendarId?: string;
  reply?: boolean;
  write?: boolean;
  subject?: string;
  tz?: string;
  retried?: boolean;
}

export interface EventsRsvpResult {
  email: string;
  write?: EventWriteJSON;
  replyTaskId?: string;
}

// `data` is the contact editor's shape (name, emailAddresses, phoneNumbers, addresses, company,
// title, birthdays, urls, nicknames, relations, notes, photoURL).
export interface ContactDetailsJSON {
  contact: any;
  metadata: { origin: string; readonly: boolean };
  data: any;
  taskId?: string;
}

export interface ContactsImportResult {
  contactIds: string[];
  taskIds: string[];
}
