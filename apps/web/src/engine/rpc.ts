// The engine's method map, generated from crates/tern-ipc (packages/client, `@tern/client`)
// and re-exported under the names the web has used, plus the JSON-RPC 2.0 envelope.
import type {
  AccountState,
  Capabilities,
  QueryParams as AnyQueryParams,
  QueryResult as AnyQueryResult,
  QueryClassName,
  RpcMethods as AllRpcMethods,
  RpcNotifications as AllRpcNotifications,
  SettingsValues as SettingsGetResult,
} from '@tern/client';
import { ErrorCode, Feature } from '@tern/client';
import type { Delta } from './types';

export type {
  AccountInfo as EngineAccountSummary,
  AccountListResult,
  AccountStateParams,
  AccountStatus as AccountListEntry,
  AccountStatus as EngineAccountStatus,
  AccountTestResult,
  AiCompletion,
  AiDeltaParams,
  AiDraftReplyParams,
  AiModelInfo,
  AiProviderParams,
  AiSuggestion,
  AiSuggestRepliesParams,
  AiSummarizeParams,
  AiTestReport,
  AiTokenUsage,
  AiTriageListParams,
  AiTriageListResult,
  AiTriageProgressParams,
  AiTriageRunParams,
  AiTriageRunResult,
  AiUsageRow,
  Capabilities as EngineCapabilities,
  Capability as EngineCapability,
  CategoriesListParams,
  CategoryCounts as CategoryCount,
  ContactsSearchParams,
  ContactsSearchResult,
  Empty as EmptyResult,
  EngineStatusResult as EngineStatus,
  FoldersListResult,
  HelloParams,
  HelloResult,
  IntegrationConfig,
  IntegrationParams,
  IntegrationTestReport,
  LabelsListResult,
  MatcherJson as MatcherJSON,
  MessagesGetParams,
  MessagesGetResult,
  MessagesUnsubscribeParams,
  MessagesUnsubscribeResult,
  ProviderErrorJson as AiErrorJSON,
  QueryClassName as QueryClass,
  QueryOp,
  SearchParams,
  SearchResult,
  SyncErrorJson as SyncErrorJSON,
  ThreadsGetParams,
  ThreadsGetResult,
  ThreadsListParams,
  ThreadsListResult,
  ThreadsSnoozeParams,
  ThreadsSnoozeResult,
  ThreadsUnsnoozeParams,
} from '@tern/client';
export { PROTOCOL_VERSION, UnsubscribeMethod } from '@tern/client';
export type { SettingsGetResult };

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

/** tern-ipc `code`; `NotAllowedRelayed` is the web's name for `ForbiddenRelayed`. */
export const RpcErrorCode = {
  ...ErrorCode,
  NotAllowedRelayed: ErrorCode.ForbiddenRelayed,
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

export type AccountRunState = AccountState;

/** `hello.features` (tern-ipc `feature`). */
export const EngineFeature = Feature;
export type EngineFeature = Feature;

/**
 * Engine settings (DCP stage 3) by config key path, e.g. `core.sending.undoSend`. A key with
 * no stored value is absent; in `settings.set` and `settings.changed` null clears one.
 */
export type SettingsValues = SettingsGetResult['values'];

/** `query` with its class narrowed. */
export type QueryParams<C extends QueryClassName = QueryClassName> = Omit<
  AnyQueryParams,
  'class'
> & { class: C };

/** `query`'s result with its rows typed by the caller. */
export type QueryResult<T = unknown> = Omit<AnyQueryResult, 'rows'> & { rows: T[] };

/** The params every `ai.*` call about a thread takes. */
export type AiThreadParams = Pick<
  AllRpcMethods['ai.summarize']['params'],
  'accountId' | 'threadId' | 'provider' | 'streamId'
>;

/** The methods the web calls (and the mock engine answers); add one here to call it. */
type WebMethod =
  | 'hello'
  | 'subscribe'
  | 'task.queue'
  | 'task.cancel'
  | 'sync.wake'
  | 'bodies.fetch'
  | 'calendar.sync'
  | 'engine.status'
  | 'folders.list'
  | 'labels.list'
  | 'threads.list'
  | 'threads.get'
  | 'messages.get'
  | 'search'
  | 'contacts.search'
  | 'query'
  | 'threads.snooze'
  | 'threads.unsnooze'
  | 'messages.unsubscribe'
  | 'account.add'
  | 'account.remove'
  | 'account.test'
  | 'account.reset'
  | 'engine.shutdown'
  | 'account.list'
  | 'settings.get'
  | 'settings.set'
  | 'integrations.translate'
  | 'integrations.grammar'
  | 'integrations.enrich'
  | 'integrations.test'
  | 'ai.models'
  | 'ai.test'
  | 'ai.summarize'
  | 'ai.suggestReplies'
  | 'ai.draftReply'
  | 'ai.cancel'
  | 'ai.usage'
  | 'ai.triage.run'
  | 'ai.triage.list';

export type RpcMethods = Pick<AllRpcMethods, WebMethod>;
export type RpcMethodName = keyof RpcMethods;
export type RpcParams<M extends RpcMethodName> = RpcMethods[M]['params'];
export type RpcResult<M extends RpcMethodName> = RpcMethods[M]['result'];

/** Lets `call('engine.status')` omit params while every other method requires them. */
export type RpcParamsArg<M extends RpcMethodName> =
  RpcParams<M> extends undefined ? [params?: undefined] : [params: RpcParams<M>];

/** The notifications the web handles, `delta` typed by model class. */
export interface RpcNotifications extends Pick<
  AllRpcNotifications,
  'account.state' | 'settings.changed' | 'ai.delta' | 'ai.triage.progress'
> {
  delta: Delta;
  'capabilities.changed': Capabilities;
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
