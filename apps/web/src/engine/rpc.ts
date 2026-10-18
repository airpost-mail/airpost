// JSON-RPC 2.0 envelope and the engine's method map (docs/engine-protocol-plan.md, 1.2).
import type {
  AccountJSON,
  ContactJSON,
  Delta,
  FileJSON,
  FolderJSON,
  LabelJSON,
  MessageJSON,
  TaskJSON,
  ThreadJSON,
} from './types';

export const PROTOCOL_VERSION = 1;

export type RpcId = number | string;

export interface RpcRequest<M extends string = string, P = unknown> {
  jsonrpc: '2.0';
  id: RpcId;
  method: M;
  params?: P;
}

export interface RpcErrorObject {
  code: number;
  message: string;
  data?: unknown;
}

export type RpcResponse<R = unknown> =
  | { jsonrpc: '2.0'; id: RpcId; result: R }
  | { jsonrpc: '2.0'; id: RpcId | null; error: RpcErrorObject };

export interface RpcNotification<M extends string = string, P = unknown> {
  jsonrpc: '2.0';
  method: M;
  params: P;
}

export const RpcErrorCode = {
  ParseError: -32700,
  InvalidRequest: -32600,
  MethodNotFound: -32601,
  InvalidParams: -32602,
  InternalError: -32603,
  UnsupportedProtocol: -32000,
  UnknownAccount: -32001,
  TaskRejected: -32002,
  HelloRequired: -32003,
  NotFound: -32004,
  AccountStopped: -32005,
  AccountExists: -32006,
  WrongChannel: -32007,
  /** The gateway or engine refused a method on a relayed connection (plan G6). */
  NotAllowedRelayed: -32008,
  /** `integrations.*`: the provider failed; the message says how, `data` is the error. */
  IntegrationFailed: -32010,
  /** A worker lane of the engine is full; try again later. */
  EngineBusy: -32012,
} as const;

export class RpcError extends Error {
  readonly code: number;
  readonly data?: unknown;

  constructor({ code, message, data }: RpcErrorObject) {
    super(message);
    this.name = 'RpcError';
    this.code = code;
    this.data = data;
  }
}

export type EmptyResult = Record<string, never>;

export type AccountRunState = 'running' | 'stopped' | 'starting' | 'removed';

/** The `SyncError` JSON that stopped an account. */
export interface SyncErrorJSON {
  key?: string;
  debuginfo?: string;
  retryable?: boolean;
  [key: string]: unknown;
}

export interface EngineAccountSummary {
  accountId: string;
  email: string;
  state: AccountRunState;
  error?: SyncErrorJSON;
}

/** Auth is on the WebSocket upgrade (the gateway's pairing cookie), not in `hello`. */
export interface HelloParams {
  protocol: number;
  client: string;
}

export interface HelloResult {
  protocol: number;
  engine: string;
  pid: number;
  accounts: EngineAccountSummary[];
  /** What this engine has beyond protocol 1 (ms-ipc `feature`); absent on older engines. */
  features?: string[];
  /** With `EngineFeature.Capabilities`; `capabilities.changed` carries later values. */
  capabilities?: EngineCapabilities;
}

/** One entry of `capabilities` (ms-ipc `Capability`, DCP 3.1). */
export interface EngineCapability {
  available: boolean;
  provider?: string;
  /** `request`: the client sends the key with each call. */
  credentials?: string;
  /** Why it's unavailable: `not-set-up`, `missing-url`, `needs-tern-server`, … */
  reason?: string;
}

/**
 * `hello.capabilities` and `capabilities.changed` (ms-ipc `Capabilities`): the integrations
 * by kind (`translation`, `grammar`, `enrichment`) and the Tern server's features
 * (`readReceipts`, `linkTracking`, `threadSharing`, `activitySharing`, `metadataSync`).
 * Groups a newer engine adds are kept as they came.
 */
export interface EngineCapabilities {
  integrations?: Record<string, EngineCapability>;
  server?: Record<string, EngineCapability>;
  [group: string]: Record<string, EngineCapability> | undefined;
}

/**
 * Engine settings (DCP stage 3) by config key path, e.g. `core.sending.undoSend`. A key with
 * no stored value is absent; in `settings.set` and `settings.changed` null clears one.
 */
export type SettingsValues = Record<string, unknown>;

export interface SettingsGetResult {
  values: SettingsValues;
  storeId?: string;
}

/** `hello.features` the web acts on. */
export const EngineFeature = {
  /** `query` (DCP stage 7). */
  Query: 'query',
  /**
   * The search grammar (DCP stage 9) in `search {grammar: true, tz, now}` and
   * `query.search`, and `search {accounts}`.
   */
  SearchGrammar: 'search-grammar',
  /** `files.*` (DCP stage 16): the gateway's file routes work. */
  Files: 'files',
  /** `threads.snooze` and `threads.unsnooze` (DCP stage 15). */
  Snooze: 'snooze',
  /** `messages.unsubscribe` (DCP stage 15). */
  Unsubscribe: 'unsubscribe',
  /**
   * `settings.get`, `settings.set` and `settings.changed` (DCP stage 3). A device without
   * the admin scope may set only the keys in ms-ipc `RELAYED_SETTINGS_KEYS`.
   */
  Settings: 'settings',
  /** The engine sends due send-later drafts and returns snoozed threads (DCP stage 4). */
  ExpirationExecutor: 'expiration-executor',
  /** `hello.capabilities` and `capabilities.changed` (DCP stage 5). */
  Capabilities: 'capabilities',
  /** `integrations.*` on the engine (DCP stage 5). */
  Integrations: 'integrations',
} as const;

/** ms-ipc `EngineStatusResult`. */
export interface EngineStatus {
  protocol: number;
  engine: string;
  pid: number;
  uptimeSecs: number;
  accounts: EngineAccountStatus[];
}

export interface EngineAccountStatus {
  accountId: string;
  email: string;
  state: AccountRunState;
  /** The last ProcessState the account sent: true while it cannot reach its server. */
  connectionError: boolean;
  error?: SyncErrorJSON;
}

/**
 * One `account.list` row. `account` is the non-secret Account JSON copied through an
 * allowlist (docs/web-gateway-plan.md section 3); an engine without it sends none.
 */
export interface AccountListEntry extends EngineAccountStatus {
  account?: Partial<AccountJSON>;
}

export interface AccountListResult {
  accounts: AccountListEntry[];
}

// Read API params and results, as in crates/ms-ipc/src/read.rs. A `limit` defaults to 50
// and is clamped to 1..=500 by the engine.

export interface CategoriesListParams {
  accountId: string;
}

export interface CategoryCount {
  unread: number;
  total: number;
}

/** `counts` is keyed by category id and leaves out categories without a ThreadCounts row. */
export interface FoldersListResult {
  folders: FolderJSON[];
  counts: Record<string, CategoryCount>;
}

export interface LabelsListResult {
  labels: LabelJSON[];
  counts: Record<string, CategoryCount>;
}

/**
 * At most one of `folderId`, `labelId` and `role`; none lists every thread of the account.
 * Outside spam and trash a category lists only threads in all mail.
 */
export interface ThreadsListParams {
  accountId: string;
  folderId?: string;
  labelId?: string;
  role?: string;
  limit?: number;
  /** The `next` of the previous page. */
  before?: string;
}

/** Newest first by `(lastMessageTimestamp, id)`. `next` is opaque; null on the last page. */
export interface ThreadsListResult {
  threads: ThreadJSON[];
  next: string | null;
}

export interface ThreadsGetParams {
  accountId: string;
  id: string;
}

/** The thread and its messages, oldest first, without bodies. */
export interface ThreadsGetResult {
  thread: ThreadJSON;
  messages: MessageJSON[];
}

export interface MessagesGetParams {
  accountId: string;
  id: string;
  /** Include the body from MessageBody. */
  body?: boolean;
}

/** `body` is null unless asked for and already fetched (`bodies.fetch` fetches it). */
export interface MessagesGetResult {
  message: MessageJSON;
  body: string | null;
  files: FileJSON[];
}

/**
 * `search`. Without `grammar` the query is plain text: each whitespace-separated term must
 * match and nothing is read as syntax. With `grammar: true` (`EngineFeature.SearchGrammar`)
 * it is the search bar's grammar, dates read in `tz` (IANA; UTC when absent) and counted back
 * from `now` (unix seconds); text that doesn't parse is `InvalidParams`. The threads are
 * `accountId`'s, `accounts`' (`SearchGrammar`), or with neither every account's.
 */
export interface SearchParams {
  accountId?: string;
  accounts?: string[] | null;
  query: string;
  limit?: number;
  grammar?: boolean;
  tz?: string;
  now?: number;
}

/** `contacts.search`: plain text, each term matching a word prefix. */
export interface ContactsSearchParams {
  accountId: string;
  query: string;
  limit?: number;
}

/** Matching threads, newest first. */
export interface SearchResult {
  threads: ThreadJSON[];
}

/** Matching contacts that aren't hidden, most referenced first. */
export interface ContactsSearchResult {
  contacts: ContactJSON[];
}

/** `query` operators (desktop `src/flux/attributes/matcher.ts`). */
export type QueryOp =
  | '='
  | '!='
  | '<'
  | '>'
  | '<='
  | '>='
  | 'in'
  | 'not in'
  | 'contains'
  | 'containsAny'
  | 'startsWith'
  | 'like';

/** A matcher in `query.where`: attributes are the desktop model keys, not the JSON keys. */
export type MatcherJSON =
  | { attr: string; op: QueryOp; value: unknown }
  | { and: MatcherJSON[] }
  | { or: MatcherJSON[] }
  | { not: MatcherJSON[] };

/** The classes `query` reads. */
export type QueryClass =
  | 'Thread'
  | 'Message'
  | 'Folder'
  | 'Label'
  | 'Contact'
  | 'ContactGroup'
  | 'ContactBook'
  | 'Event'
  | 'Calendar'
  | 'Task'
  | 'File';

/**
 * `query` (DCP 3.2, crates/ms-ipc `QueryParams`): the JSON form of the desktop's ModelQuery.
 * `accounts` null or absent reads every account. `where` as a list means all must match. An
 * empty `order` is the class's natural order. `cursor` pages by keyset: `''` for the first
 * page, then the previous page's `nextCursor`. No limit applies when `limit` is absent.
 * Unknown keys are refused.
 */
export interface QueryParams<C extends QueryClass = QueryClass> {
  class: C;
  accounts?: string[] | null;
  where?: MatcherJSON | MatcherJSON[] | null;
  /** `grammar: true` (Thread only) needs `EngineFeature.SearchGrammar`; see `SearchParams`. */
  search?: { text: string; grammar: boolean; tz?: string; now?: number } | null;
  order?: { attr: string; dir: 'asc' | 'desc' }[];
  limit?: number | null;
  offset?: number | null;
  cursor?: string | null;
  distinct?: boolean;
  /** Joined data, `body` on Message. */
  include?: string[];
  idsOnly?: boolean;
  count?: boolean;
}

/**
 * `rows` always (empty for `idsOnly` and `count`); `ids` for `idsOnly`, `count` for `count`,
 * `nextCursor` for a cursor query (null after the last page).
 */
export interface QueryResult<T = unknown> {
  rows: T[];
  ids?: string[];
  count?: number;
  nextCursor?: string | null;
}

// Mail actions, as in crates/ms-ipc/src/mail.rs.

/**
 * `threads.snooze`: the threads move to Snoozed and get the `thread-snooze` metadata that
 * brings them back at `until` (unix seconds, in the future), as the desktop's snooze does.
 */
export interface ThreadsSnoozeParams {
  accountId: string;
  threadIds: string[];
  until: number;
}

/** `threads.unsnooze`: the snooze cleared, and what is still in Snoozed moved to the inbox. */
export interface ThreadsUnsnoozeParams {
  accountId: string;
  threadIds: string[];
}

/** The tasks queued, the move first; their local phases have run. */
export interface ThreadsSnoozeResult {
  taskIds: string[];
}

export type UnsubscribeMethod = 'one-click' | 'mailto' | 'web';

/**
 * `messages.unsubscribe`: acts on the message's `List-Unsubscribe` header. Without `method`
 * the engine picks one-click when offered, else mailto, else web; a method the header
 * doesn't offer, or no header, is `NotFound`. Links found only in the body are the client's.
 */
export interface MessagesUnsubscribeParams {
  accountId: string;
  messageId: string;
  method?: UnsubscribeMethod;
}

/**
 * one-click: `ok` when the list's server answered 2xx (the engine made the POST). mailto:
 * `ok` when the message is queued (`taskId`). web: always `ok`; the client opens `url`.
 */
export interface MessagesUnsubscribeResult {
  method: UnsubscribeMethod;
  ok: boolean;
  url?: string;
  error?: string;
  taskId?: string;
}

export interface AccountTestResult {
  ok: boolean;
  error?: string;
  log?: string;
}

/**
 * An integration provider with its settings, key included (docs/integrations-plan.md 3-4,
 * ms-integrations `ProviderConfig`).
 */
export interface IntegrationConfig {
  provider: 'libretranslate' | 'deepl' | 'languagetool' | 'peopledatalabs';
  url?: string;
  apiKey?: string;
  username?: string;
}

/** `integrations.*` params: the provider's config and the request's own fields. */
export interface IntegrationParams {
  config: IntegrationConfig;
  [key: string]: unknown;
}

/** `integrations.test`: the host that answered, and DeepL's characters used. */
export interface IntegrationTestReport {
  host: string;
  usage?: { used: number; limit?: number };
}

export interface RpcMethods {
  hello: { params: HelloParams; result: HelloResult };
  subscribe: {
    params: { accounts: string[] | null; classes: string[] | null };
    result: EmptyResult;
  };
  'task.queue': { params: { accountId: string; task: TaskJSON }; result: TaskJSON };
  'task.cancel': { params: { accountId: string; taskId: string }; result: EmptyResult };
  'sync.wake': { params: { accountId?: string }; result: EmptyResult };
  'bodies.fetch': { params: { accountId: string; ids: string[] }; result: EmptyResult };
  'calendar.sync': { params: { accountId: string }; result: { started: boolean } };
  'engine.status': { params: undefined; result: EngineStatus };

  // Phase B read API.
  'folders.list': { params: CategoriesListParams; result: FoldersListResult };
  'labels.list': { params: CategoriesListParams; result: LabelsListResult };
  'threads.list': { params: ThreadsListParams; result: ThreadsListResult };
  'threads.get': { params: ThreadsGetParams; result: ThreadsGetResult };
  'messages.get': { params: MessagesGetParams; result: MessagesGetResult };
  search: { params: SearchParams; result: SearchResult };
  'contacts.search': { params: ContactsSearchParams; result: ContactsSearchResult };
  /** Only on engines with `EngineFeature.Query`. */
  query: { params: QueryParams; result: QueryResult };

  /** Only on engines with `EngineFeature.Snooze`. */
  'threads.snooze': { params: ThreadsSnoozeParams; result: ThreadsSnoozeResult };
  'threads.unsnooze': { params: ThreadsUnsnoozeParams; result: ThreadsSnoozeResult };
  /** Only on engines with `EngineFeature.Unsubscribe`. */
  'messages.unsubscribe': {
    params: MessagesUnsubscribeParams;
    result: MessagesUnsubscribeResult;
  };

  // Phase C.
  'account.add': {
    params: { account: AccountJSON; identity?: unknown };
    result: EngineAccountSummary;
  };
  'account.remove': { params: { accountId: string }; result: EmptyResult };
  'account.test': { params: { account: AccountJSON }; result: AccountTestResult };
  'account.reset': { params: { accountId: string }; result: EmptyResult };
  'engine.shutdown': { params: undefined; result: EmptyResult };

  'account.list': { params: undefined; result: AccountListResult };

  /** Only on engines with `EngineFeature.Settings`. */
  'settings.get': { params: { keys?: string[] }; result: SettingsGetResult };
  'settings.set': { params: { values: SettingsValues }; result: EmptyResult };

  // With `EngineFeature.Integrations` (DCP stage 5), local-only: a relayed device needs the
  // admin scope. The web calls only `integrations.test`; the other results are typed when a
  // view uses them (docs/integrations-plan.md section 4).
  'integrations.translate': { params: IntegrationParams; result: unknown };
  'integrations.grammar': { params: IntegrationParams; result: unknown };
  'integrations.enrich': { params: IntegrationParams; result: unknown };
  'integrations.test': { params: IntegrationParams; result: IntegrationTestReport };
}

export type RpcMethodName = keyof RpcMethods;
export type RpcParams<M extends RpcMethodName> = RpcMethods[M]['params'];
export type RpcResult<M extends RpcMethodName> = RpcMethods[M]['result'];

/** Lets `call('engine.status')` omit params while every other method requires them. */
export type RpcParamsArg<M extends RpcMethodName> =
  RpcParams<M> extends undefined ? [params?: undefined] : [params: RpcParams<M>];

export interface AccountStateParams {
  accountId: string;
  state: AccountRunState;
  error?: string;
}

export interface RpcNotifications {
  delta: Delta;
  'account.state': AccountStateParams;
  /** Another connection's change; never the caller's own (DCP stage 3). */
  'settings.changed': { values: SettingsValues };
  'capabilities.changed': EngineCapabilities;
}

export type EngineNotification = {
  [K in keyof RpcNotifications]: { method: K; params: RpcNotifications[K] };
}[keyof RpcNotifications];

export function isResponse(msg: unknown): msg is RpcResponse {
  return typeof msg === 'object' && msg !== null && 'id' in msg && !('method' in msg);
}

export function isNotification(msg: unknown): msg is RpcNotification {
  return typeof msg === 'object' && msg !== null && 'method' in msg && !('id' in msg);
}
