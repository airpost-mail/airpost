import { LiveQueries, type LiveSource } from '@airpost/client';
import type { ConnectionState, EngineClient, Unsubscribe } from '../client';
import { generateId, nowSeconds } from '../model';
import {
  EngineFeature,
  PROTOCOL_VERSION,
  RpcError,
  RpcErrorCode,
  type EngineNotification,
  type CategoryCount,
  type HelloResult,
  type IntegrationParams,
  type IntegrationTestReport,
  type RpcMethodName,
  type RpcMethods,
  type RpcParams,
  type RpcParamsArg,
  type RpcResult,
  type SettingsValues,
  type TemplateInfo,
  type UnsubscribeMethod,
} from '../rpc';
import type { AccountJson } from '@airpost/client';
import * as tasks from './tasks';
import type { AccountJSON, Delta, FileJSON, MessageJSON, TaskJSON, ThreadJSON } from '../types';
import { applyTask } from './apply-task';
import { ChangeSet } from './changes';
import {
  THREAD_SNOOZE,
  chooseUnsubscribe,
  parseMailto,
  snoozeTasks,
  unsnoozeTasks,
  unsubscribeSendTask,
} from './mail-actions';
import { createDraft, findDraft, validateDraft } from './drafts';
import { planIntent, type MockIntent } from './intents';
import { expandEvents } from './events';
import { MockLiveQueries } from './live';
import { runQuery } from './query';
import { MockStore } from './store';
import { MockAi } from '@/features/ai/mock';

/** `templates.*` answers: the engine reads `<config>/templates`. */
const MOCK_TEMPLATES: TemplateInfo[] = [
  {
    id: 'Thanks.html',
    name: 'Thanks',
    subject: 'Thank you',
    preview: 'Thanks for getting back to me so quickly.',
    variables: [],
    body: '<div>Thanks for getting back to me so quickly.</div>',
  },
  {
    id: 'Meeting follow-up.html',
    name: 'Meeting follow-up',
    subject: '',
    preview: 'Good to talk today. As promised, here are the next steps:',
    variables: [],
    body: '<div>Good to talk today. As promised, here are the next steps:</div><ul><li></li></ul>',
  },
];

/** The classes whose changes can move a category's counts. */
const COUNTED: ReadonlySet<string> = new Set(['Thread', 'Folder', 'Label']);

/** The desktop's send-later plugin id, which undo send also writes. */
const SEND_LATER = 'send-later';

/** What the mock announces in `hello.features` unless told otherwise. */
export const MOCK_FEATURES: readonly string[] = [
  EngineFeature.Counts,
  EngineFeature.Query,
  EngineFeature.SearchGrammar,
  EngineFeature.Files,
  EngineFeature.Snooze,
  EngineFeature.Intents,
  EngineFeature.Unsubscribe,
  EngineFeature.Settings,
  EngineFeature.ExpirationExecutor,
  EngineFeature.Capabilities,
  EngineFeature.Ai,
  EngineFeature.AiTriage,
  EngineFeature.LiveQueries,
  EngineFeature.ThreadSummary,
];

/**
 * airpost-ipc `SETTINGS_KEYS` with the JSON type each takes, and the ones a device without the
 * admin scope may set (`RELAYED_SETTINGS_KEYS`); the mock always plays such a device.
 */
const SETTING_KINDS: Record<string, 'count' | 'string' | 'bool' | 'object' | 'array'> = {
  'core.sending.undoSend': 'count',
  'core.sending.defaultAccountIdForSend': 'string',
  'core.notifications.unsnoozeToTop': 'bool',
  'core.composing.html': 'bool',
  signatures: 'object',
  defaultSignatures: 'object',
  templates: 'array',
  mailRules: 'array',
  mailRulesAutoSince: 'count',
  'resurfacing.snoozeBody': 'string',
  'resurfacing.reminderBody': 'string',
  'core.integrations.translation': 'object',
  'core.integrations.grammar': 'object',
  'core.integrations.enrichment': 'object',
  'core.ai.providers': 'array',
  'core.ai.features': 'object',
  'core.ai.accounts': 'array',
  'core.ai.includeAddresses': 'bool',
  'core.ai.limits': 'object',
};
const RELAYED_SETTINGS_KEYS = [
  'core.sending.undoSend',
  'core.sending.defaultAccountIdForSend',
  'core.notifications.unsnoozeToTop',
  'core.composing.html',
  'defaultSignatures',
];

function settingAccepts(kind: string, v: unknown): boolean {
  switch (kind) {
    case 'count':
      return typeof v === 'number' && Number.isInteger(v) && v >= 0;
    case 'string':
      return typeof v === 'string';
    case 'bool':
      return typeof v === 'boolean';
    case 'array':
      return Array.isArray(v);
    default:
      return typeof v === 'object' && v !== null && !Array.isArray(v);
  }
}

/** What `hello.capabilities` says with nothing set up and no Airpost server. */
const MOCK_CAPABILITIES = {
  integrations: Object.fromEntries(
    ['translation', 'grammar', 'enrichment'].map((k) => [
      k,
      { available: false, reason: 'not-set-up' },
    ]),
  ),
  server: Object.fromEntries(
    ['readReceipts', 'linkTracking', 'threadSharing', 'activitySharing', 'metadataSync'].map(
      (k) => [k, { available: false, reason: 'needs-airpost-server' }],
    ),
  ),
};

/** Methods an engine has only when it announces the feature. */
const METHOD_FEATURE: Partial<Record<RpcMethodName, string>> = {
  query: EngineFeature.Query,
  'threads.snooze': EngineFeature.Snooze,
  'threads.unsnooze': EngineFeature.Snooze,
  'threads.archive': EngineFeature.Intents,
  'threads.trash': EngineFeature.Intents,
  'threads.spam': EngineFeature.Intents,
  'threads.notSpam': EngineFeature.Intents,
  'threads.move': EngineFeature.Intents,
  'threads.label': EngineFeature.Intents,
  'threads.markRead': EngineFeature.Intents,
  'threads.star': EngineFeature.Intents,
  'messages.unsubscribe': EngineFeature.Unsubscribe,
  'settings.get': EngineFeature.Settings,
  'settings.set': EngineFeature.Settings,
  'ai.models': EngineFeature.Ai,
  'ai.test': EngineFeature.Ai,
  'ai.summarize': EngineFeature.Ai,
  'ai.suggestReplies': EngineFeature.Ai,
  'ai.draftReply': EngineFeature.Ai,
  'ai.cancel': EngineFeature.Ai,
  'ai.usage': EngineFeature.Ai,
  'ai.triage.run': EngineFeature.AiTriage,
  'ai.triage.list': EngineFeature.AiTriage,
};

const UNSUBSCRIBE_METHODS: readonly string[] = ['one-click', 'mailto', 'web'];

export interface MockEngineOptions {
  /** `hello.features`; `[]` stands in for an engine without `query`. */
  features?: readonly string[];
  /** Artificial latency per call in ms, as a fixed value or a [min, max] range. */
  latency?: number | [number, number];
  /** Reference time for fixtures, unix seconds. */
  now?: number;
  /** Deliver a scripted incoming thread every N ms; null to disable. */
  incomingEveryMs?: number | null;
  /** A local model set up for every account and the inboxes triaged (`npm run dev`). */
  aiDemo?: boolean;
}

type Handlers = {
  [M in RpcMethodName]: (params: RpcMethods[M]['params']) => RpcResult<M> | Promise<RpcResult<M>>;
};

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/**
 * Params the read methods accept (crates/airpost-ipc/src/read.rs). The real engine's serde
 * ignores unknown keys, so a filter it doesn't know silently widens the result; the mock
 * rejects them instead so the gap shows up here.
 */
const READ_PARAMS: Partial<Record<RpcMethodName, readonly string[]>> = {
  'folders.list': ['accountId'],
  'labels.list': ['accountId'],
  'threads.list': ['accountId', 'folderId', 'labelId', 'role', 'limit', 'before'],
  'threads.get': ['accountId', 'id'],
  'messages.get': ['accountId', 'id', 'body'],
  search: ['accountId', 'query', 'limit'],
  'contacts.search': ['accountId', 'query', 'limit'],
};

/** `search` on an engine with `EngineFeature.SearchGrammar`: `accountId` is optional. */
const GRAMMAR_SEARCH_PARAMS = ['accountId', 'accounts', 'query', 'limit', 'grammar', 'tz', 'now'];

function checkParams(method: RpcMethodName, params: unknown, features: readonly string[]) {
  const grammarSearch = method === 'search' && features.includes(EngineFeature.SearchGrammar);
  const allowed = grammarSearch ? GRAMMAR_SEARCH_PARAMS : READ_PARAMS[method];
  if (!allowed) return;
  if (typeof params !== 'object' || params === null) {
    throw new RpcError({ code: RpcErrorCode.InvalidParams, message: 'invalid params' });
  }
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && !allowed.includes(k)) {
      throw new RpcError({
        code: RpcErrorCode.InvalidParams,
        message: `invalid params: ${method} has no param ${k}`,
      });
    }
  }
  const accountId = (params as { accountId?: unknown }).accountId;
  if (
    grammarSearch
      ? accountId !== undefined && typeof accountId !== 'string'
      : typeof accountId !== 'string'
  ) {
    throw new RpcError({
      code: RpcErrorCode.InvalidParams,
      message: 'invalid params: missing field `accountId`',
    });
  }
}

/** In-browser engine over fixture data. Speaks the same methods and deltas as the real one. */
export class MockEngine implements EngineClient {
  readonly kind = 'mock' as const;
  readonly store: MockStore;
  private _state: ConnectionState = 'idle';
  private subscribed: { accounts: string[] | null; classes: string[] | null } | null = null;
  private readonly notificationListeners = new Set<(n: EngineNotification) => void>();
  private readonly stateListeners = new Set<(s: ConnectionState) => void>();
  private readonly latency: [number, number];
  private readonly incomingEveryMs: number | null;
  private incomingTimer: ReturnType<typeof setInterval> | null = null;
  private readonly startedAt = Date.now();
  private readonly expiryTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly handlers: Handlers;
  private readonly announced: readonly string[];
  private _features: ReadonlySet<string> | null = null;
  /** `uploadFile`'s bytes and the account they went to, by File id. */
  private readonly uploads = new Map<string, { accountId: string; blob: Blob }>();
  /** The engine's settings store. */
  readonly settings = new Map<string, unknown>();
  private readonly ai: MockAi;
  /** The counts the last ThreadCounts deltas reported, by category id. */
  private readonly sentCounts = new Map<string, CategoryCount>();
  /** The engine's side of live queries. */
  private readonly liveSubs: MockLiveQueries;
  private _hello: HelloResult | null = null;
  private readonly helloListeners = new Set<(hello: HelloResult) => void>();
  private readonly disconnectListeners = new Set<() => void>();
  /** This tab's live queries over the mock. */
  readonly live: LiveQueries;

  constructor(opts: MockEngineOptions = {}) {
    const l = opts.latency ?? [40, 140];
    this.latency = typeof l === 'number' ? [l, l] : l;
    this.incomingEveryMs = opts.incomingEveryMs ?? null;
    this.announced = opts.features ?? MOCK_FEATURES;
    this.store = new MockStore(opts.now ?? nowSeconds());
    this.ai = new MockAi({
      store: this.store,
      settings: this.settings,
      send: (n) => {
        if (this._state === 'open') for (const l of this.notificationListeners) l(n);
      },
      queueTask: (accountId, task) => void this.queueTask(accountId, task),
      requireAccount: (id) => this.requireAccount(id),
      wait: (scale) => this.delay(scale),
    });
    if (opts.aiDemo) this.ai.seedDemo();
    this.liveSubs = new MockLiveQueries(
      (q) =>
        runQuery(this.store, q, (id) => this.requireAccount(id), {
          grammar: this.announced.includes(EngineFeature.SearchGrammar),
        }),
      (params) => {
        if (this.subscribed) {
          const n = { method: 'query.changed', params: structuredClone(params) } as const;
          for (const l of this.notificationListeners) l(n);
        }
      },
    );
    this.handlers = this.buildHandlers();
    for (const [id, c] of Object.entries(this.allCounts())) this.sentCounts.set(id, c);
    this.live = new LiveQueries(this.liveSource(), 'web-');
  }

  get hello(): HelloResult | null {
    return this._hello;
  }

  onHello(listener: (hello: HelloResult) => void): Unsubscribe {
    this.helloListeners.add(listener);
    return () => this.helloListeners.delete(listener);
  }

  onDisconnect(listener: () => void): Unsubscribe {
    this.disconnectListeners.add(listener);
    return () => this.disconnectListeners.delete(listener);
  }

  private liveSource(): LiveSource {
    const hello = () => this._hello;
    return {
      get hello() {
        return hello();
      },
      request: (method, params) =>
        this.call(method as RpcMethodName, params as never) as Promise<unknown>,
      onNotification: (l) => this.onNotification(l),
      onHello: (l) => this.onHello(l),
      onDisconnect: (l) => this.onDisconnect(l),
    };
  }

  get state() {
    return this._state;
  }

  readonly problem = null;

  get features() {
    return this._features;
  }

  retryNow() {}

  async connect(): Promise<HelloResult> {
    this.setState('connecting');
    await this.delay();
    const hello = await this.handlers.hello({ protocol: PROTOCOL_VERSION, client: 'airpost-web' });
    this._features = new Set(hello.features);
    this.liveSubs.clear();
    this._hello = hello;
    this.setState('open');
    this.subscribed = { accounts: null, classes: null };
    for (const l of [...this.helloListeners]) l(hello);
    if (this.incomingEveryMs) {
      this.incomingTimer = setInterval(() => this.deliverIncoming(), this.incomingEveryMs);
    }
    return hello;
  }

  close() {
    for (const t of this.expiryTimers.values()) clearTimeout(t);
    this.expiryTimers.clear();
    if (this.incomingTimer) clearInterval(this.incomingTimer);
    this.incomingTimer = null;
    this.subscribed = null;
    this.liveSubs.clear();
    const had = this._hello !== null;
    this._hello = null;
    this.setState('closed');
    if (had) for (const l of [...this.disconnectListeners]) l();
  }

  async call<M extends RpcMethodName>(method: M, ...args: RpcParamsArg<M>): Promise<RpcResult<M>> {
    if (this._state !== 'open') {
      throw new RpcError({ code: RpcErrorCode.InternalError, message: 'engine is not connected' });
    }
    await this.delay();
    const handler = this.handlers[method] as
      ((p: unknown) => RpcResult<M> | Promise<RpcResult<M>>) | undefined;
    const needs = METHOD_FEATURE[method];
    if (!handler || (needs && !this.announced.includes(needs))) {
      throw new RpcError({
        code: RpcErrorCode.MethodNotFound,
        message: `unknown method ${method}`,
      });
    }
    checkParams(method, args[0], this.announced);
    // Results cross a (pretend) wire: never hand out references into the store.
    return structuredClone(await handler(args[0]));
  }

  onNotification(listener: (n: EngineNotification) => void): Unsubscribe {
    this.notificationListeners.add(listener);
    return () => this.notificationListeners.delete(listener);
  }

  onStateChange(listener: (s: ConnectionState) => void): Unsubscribe {
    this.stateListeners.add(listener);
    return () => this.stateListeners.delete(listener);
  }

  fileUrl(file: FileJSON): string | null {
    // Uploads keep their bytes; everything else gets placeholder bytes so links work.
    const blob =
      this.uploads.get(file.id)?.blob ??
      new Blob([`Mock attachment ${file.filename} (${file.size} bytes)\n`], {
        type: 'text/plain',
      });
    return typeof URL.createObjectURL === 'function' ? URL.createObjectURL(blob) : null;
  }

  async uploadFile(accountId: string, file: Blob, name: string): Promise<FileJSON> {
    await this.delay(2);
    const stored: FileJSON = {
      id: generateId(),
      aid: accountId,
      v: 0,
      __cls: 'File',
      messageId: '',
      contentId: null,
      contentType: file.type || 'application/octet-stream',
      filename: name,
      size: file.size,
    };
    this.uploads.set(stored.id, { accountId, blob: file });
    return stored;
  }

  async prepareFile(file: FileJSON): Promise<void> {
    await this.delay();
    // As the engine's files.get: an upload is found only under the account it was put to.
    const upload = this.uploads.get(file.id);
    if (upload && upload.accountId !== file.aid)
      throw new Error(`${file.filename} isn't available.`);
  }

  /**
   * A settings change another connection made (the desktop's): stored, and
   * `settings.changed` sent to this one. For tests and the dev console.
   */
  changeSettings(values: SettingsValues) {
    for (const [k, v] of Object.entries(values)) {
      if (v === null) this.settings.delete(k);
      else this.settings.set(k, v);
    }
    if (!this.subscribed) return;
    const n: EngineNotification = {
      method: 'settings.changed',
      params: { values: structuredClone(values) },
    };
    for (const l of this.notificationListeners) l(n);
  }

  /** Delivers the next scripted incoming thread. Exposed for tests and the dev console. */
  deliverIncoming(accountId?: string): boolean {
    const changes = this.store.deliverIncoming(nowSeconds(), accountId);
    if (!changes) return false;
    this.emit(changes);
    this.ai.newMail(changes.toDeltas());
    return true;
  }

  // -------------------------------------------------------------- internals

  private setState(s: ConnectionState) {
    this._state = s;
    for (const l of this.stateListeners) l(s);
  }

  private delay(scale = 1) {
    const [min, max] = this.latency;
    const ms = (min + Math.random() * (max - min)) * scale;
    return ms > 0 ? sleep(ms) : Promise.resolve();
  }

  private requireAccount(accountId: string | undefined) {
    if (!accountId || !this.store.account(accountId)) {
      throw new RpcError({
        code: RpcErrorCode.UnknownAccount,
        message: `unknown account ${accountId}`,
      });
    }
    return accountId;
  }

  /** Flushes deltas after a short delay, like the engine's 5 ms DeltaStream timer. */
  private emit(changes: ChangeSet) {
    const deltas = changes.toDeltas();
    if (deltas.length === 0) return;
    if (deltas.some((d) => COUNTED.has(d.modelClass))) deltas.push(...this.countDeltas());
    void this.delay(0.5).then(() => {
      for (const d of deltas) this.notify(d);
      this.liveSubs.changed(new Set(deltas.map((d) => d.modelClass)));
    });
  }

  private allCounts(): Record<string, CategoryCount> {
    const out: Record<string, CategoryCount> = {};
    for (const id of this.store.accounts.keys()) {
      Object.assign(out, this.store.counts(id, 'Folder'), this.store.counts(id, 'Label'));
    }
    return out;
  }

  /** ThreadCounts deltas for the categories whose counts changed, as airpost-sync sends them. */
  private countDeltas(): Delta[] {
    const now = this.allCounts();
    const out: Delta<'ThreadCounts'>[] = [];
    const add = (type: 'persist' | 'unpersist', categoryId: string, c?: CategoryCount) => {
      const aid = this.store.categories.get(categoryId)?.aid ?? '';
      let d = out.find((x) => x.type === type && x.accountId === aid);
      if (!d) out.push((d = { accountId: aid, modelClass: 'ThreadCounts', type, modelJSONs: [] }));
      d.modelJSONs.push({ id: categoryId, categoryId, ...c });
    };
    for (const [id, c] of Object.entries(now)) {
      const before = this.sentCounts.get(id);
      if (before?.unread === c.unread && before.total === c.total) continue;
      this.sentCounts.set(id, c);
      add('persist', id, c);
    }
    for (const id of [...this.sentCounts.keys()]) {
      if (now[id]) continue;
      this.sentCounts.delete(id);
      add('unpersist', id);
    }
    return out;
  }

  private notify(delta: Delta) {
    const sub = this.subscribed;
    if (!sub) return;
    if (sub.accounts && !sub.accounts.includes(delta.accountId)) return;
    if (sub.classes && !sub.classes.includes(delta.modelClass)) return;
    const n: EngineNotification = { method: 'delta', params: structuredClone(delta) };
    for (const l of this.notificationListeners) l(n);
  }

  private queueTask(accountId: string, input: TaskJSON): TaskJSON {
    this.requireAccount(accountId);
    const task = structuredClone(input);
    task.aid = accountId;
    const outcome = applyTask(this.store, task);
    this.watchExpiration(task);
    task.status = 'remote';
    task.v = (task.v ?? 0) + 1;
    this.store.tasks.set(task.id, task);
    outcome.changes.persist('Task', task);
    this.emit(outcome.changes);

    setTimeout(() => {
      const current = this.store.tasks.get(task.id);
      if (!current || current.status === 'cancelled') return;
      const done = outcome.remote?.(current) ?? new ChangeSet(accountId);
      current.status = 'complete';
      current.v = (current.v ?? 0) + 1;
      if (outcome.error) current.error = outcome.error;
      done.persist('Task', current);
      this.emit(done);
    }, outcome.remoteMs);

    return structuredClone(task);
  }

  /** The model a metadata task wrote, if any. */
  private metadataTarget(task: TaskJSON): ThreadJSON | MessageJSON | undefined {
    const s = this.store;
    if (task.__cls === 'SyncbackDraftTask') {
      return [...s.messages.values()].find((m) => m.draft && m.hMsgId === task.headerMessageId);
    }
    if (task.__cls === 'SyncbackMetadataTask') {
      return task.modelClassName.toLowerCase() === 'thread'
        ? s.threads.get(task.modelId)
        : s.messages.get(task.modelId);
    }
    return undefined;
  }

  /**
   * Fires a `metadata-expiration` delta when the model's earliest metadata `expiration`
   * passes, like airpost-sync's metadata_expiration worker.
   */
  private watchExpiration(task: TaskJSON) {
    const model = this.metadataTarget(task);
    if (!model) return;
    const key = `${model.__cls}:${model.id}`;
    clearTimeout(this.expiryTimers.get(key));
    this.expiryTimers.delete(key);
    const expiration = (m: ThreadJSON | MessageJSON) =>
      Math.min(
        ...(m.metadata ?? []).map((x) => {
          const exp = (x.value as { expiration?: unknown } | null)?.expiration;
          return typeof exp === 'number' ? exp : Infinity;
        }),
      );
    const at = expiration(model);
    if (!Number.isFinite(at)) return;
    const ms = Math.min(Math.max(0, at * 1000 - Date.now()), 2 ** 31 - 1);
    this.expiryTimers.set(
      key,
      setTimeout(() => {
        this.expiryTimers.delete(key);
        const s = this.store;
        const now = model.__cls === 'Thread' ? s.threads.get(model.id) : s.messages.get(model.id);
        if (!now || expiration(now) > nowSeconds()) return;
        if (now.__cls === 'Message' && this.executeSendLater(now)) return;
        this.notify({
          accountId: now.aid,
          modelClass: now.__cls,
          type: 'metadata-expiration',
          modelJSONs: [now.__cls === 'Message' ? s.wire(now) : structuredClone(now)],
        } as Delta);
      }, ms),
    );
  }

  /**
   * With `expiration-executor` announced, a due `send-later` row is the engine's: cleared,
   * then the draft sent (crates/airpost-sync/src/expiration_executor.rs). Returns whether it was
   * one.
   */
  private executeSendLater(m: MessageJSON): boolean {
    if (!this.announced.includes(EngineFeature.ExpirationExecutor)) return false;
    const entry = m.metadata?.find((x) => x.pluginId === SEND_LATER);
    const exp = (entry?.value as { expiration?: unknown } | null)?.expiration;
    if (typeof exp !== 'number' || exp > nowSeconds()) return false;
    const cleared = tasks.syncbackMetadata({
      accountId: m.aid,
      pluginId: SEND_LATER,
      modelId: m.id,
      modelClassName: 'Message',
      modelHeaderMessageId: m.hMsgId,
      value: { expiration: null },
    });
    this.queueTask(m.aid, cleared);
    if (!m.draft) return true;
    const draft = this.store.messages.get(m.id) ?? m;
    const body = this.store.body(m.id) ?? '';
    this.queueTask(m.aid, tasks.sendDraft({ ...this.store.wire(draft), body }));
    return true;
  }

  private sendLaterTask(d: MessageJSON, value: unknown): TaskJSON {
    return tasks.syncbackMetadata({
      accountId: d.aid,
      pluginId: SEND_LATER,
      modelId: d.id,
      modelClassName: 'Message',
      modelHeaderMessageId: d.hMsgId,
      value,
    });
  }

  /** Queues `planned` in order and returns their ids. */
  private queueAll(accountId: string, planned: TaskJSON[]): { taskIds: string[] } {
    return { taskIds: planned.map((t) => this.queueTask(accountId, t).id) };
  }

  /** A `threads.*` intent: every account's tasks, then the threads as they are after them. */
  private intent<M extends MockIntent>(method: M, params: RpcParams<M>) {
    const planned = planIntent(this.store, method, params as never);
    const taskIds = planned.flatMap(([aid, list]) => this.queueAll(aid, list).taskIds);
    const ids: string[] = (params as { threadIds?: string[] }).threadIds ?? [];
    const models = ids.flatMap((id) => {
      const t = this.store.threads.get(id);
      return t ? [structuredClone(t)] : [];
    });
    return { taskIds, undo: null, models, unchanged: [] };
  }

  /**
   * Brings a snoozed thread back at `until` if its snooze is still that one, as the
   * engine's expiration executor does.
   */
  private watchSnooze(accountId: string, threadIds: string[], until: number) {
    for (const id of threadIds) {
      const key = `snooze:${id}`;
      clearTimeout(this.expiryTimers.get(key));
      const ms = Math.min(Math.max(0, until * 1000 - Date.now()), 2 ** 31 - 1);
      this.expiryTimers.set(
        key,
        setTimeout(() => {
          this.expiryTimers.delete(key);
          const t = this.store.threads.get(id);
          const entry = t?.metadata?.find((m) => m.pluginId === THREAD_SNOOZE);
          const exp = (entry?.value as { expiration?: unknown } | null)?.expiration;
          if (!t || exp !== until) return;
          this.queueAll(accountId, unsnoozeTasks(this.store, accountId, [id]));
        }, ms),
      );
    }
  }

  private unsubscribe(p: RpcMethods['messages.unsubscribe']['params']) {
    const accountId = this.requireAccount(p.accountId);
    if (p.method != null && !UNSUBSCRIBE_METHODS.includes(p.method)) {
      throw new RpcError({
        code: RpcErrorCode.InvalidParams,
        message: 'invalid params: method must be one of one-click, mailto and web',
      });
    }
    const m = this.store.messages.get(p.messageId);
    if (!m || m.aid !== accountId) {
      throw new RpcError({ code: RpcErrorCode.NotFound, message: `no message ${p.messageId}` });
    }
    const chosen = chooseUnsubscribe(
      m.hListUnsub,
      m.hListUnsubPost,
      (p.method ?? undefined) as UnsubscribeMethod | undefined,
    );
    if (!chosen) {
      throw new RpcError({
        code: RpcErrorCode.NotFound,
        message: `message ${m.id} has no ${p.method ?? 'unsubscribe link'} in a List-Unsubscribe header`,
      });
    }
    const { method, url } = chosen;
    if (method !== 'mailto') return { method, ok: true, url };
    try {
      const task = unsubscribeSendTask(this.store.account(accountId)!, m, parseMailto(url));
      return { method, ok: true, url, taskId: this.queueTask(accountId, task).id };
    } catch (e) {
      return { method, ok: false, url, error: (e as Error).message };
    }
  }

  private buildHandlers(): Handlers {
    const s = this.store;
    const accountsSummary = () =>
      [...s.accounts.values()].map((a) => ({
        accountId: a.id,
        email: a.emailAddress,
        state: 'running' as const,
      }));

    return {
      hello: (p) => {
        if (p.protocol !== PROTOCOL_VERSION) {
          throw new RpcError({
            code: RpcErrorCode.UnsupportedProtocol,
            message: `unsupported protocol ${p.protocol}; accepts ${PROTOCOL_VERSION}`,
          });
        }
        return {
          protocol: PROTOCOL_VERSION,
          engine: '0.0.0-mock',
          pid: 0,
          accounts: accountsSummary(),
          features: [...this.announced],
          ...(this.announced.includes(EngineFeature.Capabilities)
            ? { capabilities: structuredClone(MOCK_CAPABILITIES) }
            : {}),
        };
      },
      subscribe: (p) => {
        this.subscribed = { accounts: p.accounts ?? null, classes: p.classes ?? null };
        // The mock's deltas carry no seq, so there is nothing to resume from.
        return { feed: 'mock', seq: 0 };
      },
      'query.subscribe': (p) => this.liveSubs.subscribe(p) as RpcResult<'query.subscribe'>,
      'query.setWindow': (p) => this.liveSubs.setWindow(p) as RpcResult<'query.setWindow'>,
      'query.unsubscribe': (p) => this.liveSubs.unsubscribe(p),
      'task.queue': (p) => this.queueTask(p.accountId, p.task),
      'task.cancel': (p) => {
        this.requireAccount(p.accountId);
        const t = s.tasks.get(p.taskId);
        if (t && t.status !== 'complete') {
          // A real cancel only stops the remote phase; the local changes stay.
          t.status = 'cancelled';
          t.v = (t.v ?? 0) + 1;
          const c = new ChangeSet(p.accountId);
          c.persist('Task', t);
          this.emit(c);
        }
        return {};
      },
      'sync.wake': (p) => {
        if (p.accountId) this.requireAccount(p.accountId);
        this.deliverIncoming(p.accountId ?? undefined);
        return {};
      },
      'bodies.fetch': (p) => {
        this.requireAccount(p.accountId);
        const c = new ChangeSet(p.accountId);
        for (const id of p.ids) {
          const m = s.messages.get(id);
          if (!m || m.aid !== p.accountId) continue;
          s.loadedBodies.add(id);
          c.persist('Message', s.withBody(m));
        }
        this.emit(c);
        return {};
      },
      'calendar.sync': (p) => {
        this.requireAccount(p.accountId);
        // The fixtures are all there is; nothing to fetch.
        return { started: false };
      },
      'engine.status': () => ({
        protocol: PROTOCOL_VERSION,
        engine: '0.0.0-mock',
        pid: 0,
        uptimeSecs: Math.floor((Date.now() - this.startedAt) / 1000),
        accounts: accountsSummary().map((a) => ({ ...a, connectionError: false })),
      }),
      'folders.list': (p) => {
        const id = this.requireAccount(p.accountId);
        return { folders: s.listCategories(id, 'Folder'), counts: s.counts(id, 'Folder') };
      },
      'labels.list': (p) => {
        const id = this.requireAccount(p.accountId);
        return { labels: s.listCategories(id, 'Label'), counts: s.counts(id, 'Label') };
      },
      'counts.get': (p) => {
        const ids = (p.accounts ?? [...s.accounts.keys()]).map((a) => this.requireAccount(a));
        const counts: Record<string, CategoryCount> = {};
        for (const id of ids) Object.assign(counts, s.counts(id, 'Folder'), s.counts(id, 'Label'));
        return { counts };
      },
      'events.expand': (p) => expandEvents(s, p),
      'draft.create': (p) => {
        if (p.save !== false) throw invalidParams('the mock builds drafts with save: false only');
        const id = this.requireAccount(p.accountId ?? [...s.accounts.keys()][0]);
        return createDraft(s, p, s.account(id)!);
      },
      'draft.validate': (p) =>
        validateDraft(s, { ...p, accountId: this.requireAccount(p.accountId) }),
      'draft.send': (p) => {
        const accountId = this.requireAccount(p.accountId);
        const d = findDraft(s, accountId, p.headerMessageId);
        if (!p.delayMs) {
          const body = s.body(d.id) ?? '';
          return { taskId: this.queueTask(accountId, tasks.sendDraft({ ...s.wire(d), body })).id };
        }
        // The engine's clock; its executor sends it then (`executeSendLater`).
        const expiration = Math.ceil((Date.now() + p.delayMs) / 1000);
        this.queueTask(accountId, this.sendLaterTask(d, { expiration, isUndoSend: true }));
        return { scheduledAt: expiration };
      },
      'draft.cancelSend': (p) => {
        const accountId = this.requireAccount(p.accountId);
        const d = findDraft(s, accountId, p.headerMessageId);
        const pending = d.metadata?.find((m) => m.pluginId === SEND_LATER)?.value as
          { expiration?: number | null } | undefined;
        if (typeof pending?.expiration !== 'number') return { cancelled: false };
        this.queueTask(accountId, this.sendLaterTask(d, { expiration: null, isUndoSend: true }));
        return { cancelled: true };
      },
      'templates.list': () => ({ templates: MOCK_TEMPLATES.map(({ body: _, ...t }) => t) }),
      'templates.get': (p) => {
        const template = MOCK_TEMPLATES.find((t) => t.name === p.name);
        if (!template) {
          throw new RpcError({ code: RpcErrorCode.NotFound, message: `no template ${p.name}` });
        }
        return { template };
      },
      'threads.list': (p) => {
        this.requireAccount(p.accountId);
        return s.listThreads(p);
      },
      'threads.get': (p) => {
        this.requireAccount(p.accountId);
        return s.getThread(p);
      },
      'messages.get': (p) => {
        this.requireAccount(p.accountId);
        return s.getMessage(p);
      },
      search: (p) => {
        if (p.accountId !== undefined && p.accounts) {
          throw new RpcError({
            code: RpcErrorCode.InvalidParams,
            message: 'invalid params: accountId and accounts exclude each other',
          });
        }
        const accounts =
          p.accountId != null
            ? [this.requireAccount(p.accountId)]
            : (p.accounts ?? [...s.accounts.keys()]).map((a) => this.requireAccount(a));
        return s.search(p, accounts);
      },
      'contacts.search': (p) => {
        this.requireAccount(p.accountId);
        return s.searchContacts(p);
      },
      query: (p) =>
        runQuery(s, p, (id) => this.requireAccount(id), {
          grammar: this.announced.includes(EngineFeature.SearchGrammar),
        }) as RpcResult<'query'>,
      'account.add': (p) => {
        // The engine takes `account.add` from a device paired with the admin scope. The mock
        // accepts the account but has no mail for it.
        // The web's account form sends every field.
        const account = structuredClone(p.account) as AccountJSON;
        s.accounts.set(account.id, account);
        const c = new ChangeSet(account.id);
        c.persist('Account', account);
        this.emit(c);
        return { accountId: p.account.id, email: p.account.emailAddress, state: 'running' };
      },
      'account.remove': (p) => {
        const a = s.account(this.requireAccount(p.accountId))!;
        s.accounts.delete(a.id);
        const c = new ChangeSet(a.id);
        c.unpersist('Account', a);
        this.emit(c);
        return {};
      },
      'account.test': (p) => {
        // The engine runs its IMAP/SMTP checks (admin scope from a relayed device).
        const ok = !!p.account.settings?.imap_host || p.account.provider === 'gmail';
        return ok ? { error: null, log: '' } : { error: 'IMAP host is required', log: '' };
      },
      'account.reset': (p) => {
        this.requireAccount(p.accountId);
        return {};
      },
      'engine.shutdown': () => {
        setTimeout(() => this.close(), 0);
        return {};
      },
      'account.list': () => ({
        accounts: [...s.accounts.values()].map((a) => ({
          accountId: a.id,
          email: a.emailAddress,
          state: 'running' as const,
          connectionError: false,
          account: redactAccount(a),
        })),
      }),
      'threads.snooze': (p) => {
        const accountId = this.requireAccount(p.accountId);
        const result = this.queueAll(accountId, snoozeTasks(s, accountId, p.threadIds, p.until));
        this.watchSnooze(accountId, p.threadIds, p.until);
        return result;
      },
      'threads.unsnooze': (p) => {
        const accountId = this.requireAccount(p.accountId);
        return this.queueAll(accountId, unsnoozeTasks(s, accountId, p.threadIds));
      },
      'threads.archive': (p) => this.intent('threads.archive', p),
      'threads.trash': (p) => this.intent('threads.trash', p),
      'threads.spam': (p) => this.intent('threads.spam', p),
      'threads.notSpam': (p) => this.intent('threads.notSpam', p),
      'threads.move': (p) => this.intent('threads.move', p),
      'threads.label': (p) => this.intent('threads.label', p),
      'threads.markRead': (p) => this.intent('threads.markRead', p),
      'threads.star': (p) => this.intent('threads.star', p),
      // The mock keeps no undo log.
      'undo.apply': () => {
        throw new RpcError({ code: RpcErrorCode.NotFound, message: 'nothing to undo' });
      },
      'messages.unsubscribe': (p) => this.unsubscribe(p),
      'settings.get': (p) => ({
        values: Object.fromEntries(
          [...this.settings].filter(([k]) => !p?.keys || p.keys.includes(k)),
        ),
        storeId: 'mock-store',
      }),
      'settings.set': (p) => {
        const values = p?.values ?? {};
        for (const [k, v] of Object.entries(values)) {
          const kind = SETTING_KINDS[k];
          if (!kind) throw invalidParams(`${JSON.stringify(k)} is not a setting the engine keeps`);
          if (v !== null && !settingAccepts(kind, v))
            throw invalidParams(`${k} has the wrong type`);
          if (!RELAYED_SETTINGS_KEYS.includes(k)) {
            throw new RpcError({
              code: RpcErrorCode.NotAllowedRelayed,
              message: `settings.set ${k} needs the admin scope`,
            });
          }
        }
        for (const [k, v] of Object.entries(values)) {
          if (v === null) this.settings.delete(k);
          else this.settings.set(k, v);
        }
        return {};
      },
      'integrations.translate': () => notImplemented('integrations.translate'),
      'integrations.grammar': () => notImplemented('integrations.grammar'),
      'integrations.enrich': () => notImplemented('integrations.enrich'),
      'integrations.test': (p) => mockIntegrationTest(p),
      ...this.ai.handlers(),
    };
  }
}

/**
 * The interim `account.list` copy: an allowlist of
 * fields, never a denylist, so no secret reaches the browser.
 */
export function redactAccount(a: AccountJSON): AccountJson {
  const s = a.settings;
  return {
    id: a.id,
    name: a.name,
    emailAddress: a.emailAddress,
    provider: a.provider,
    label: a.label,
    aliases: a.aliases,
    defaultAlias: a.defaultAlias,
    color: a.color,
    settings: {
      imap_host: s.imap_host,
      imap_port: s.imap_port,
      imap_username: s.imap_username,
      imap_security: s.imap_security,
      smtp_host: s.smtp_host,
      smtp_port: s.smtp_port,
      smtp_username: s.smtp_username,
      smtp_security: s.smtp_security,
    },
  };
}

const DEFAULT_HOSTS: Record<string, string> = {
  deepl: 'api-free.deepl.com',
  languagetool: 'api.languagetool.org',
  peopledatalabs: 'api.peopledatalabs.com',
};

/**
 * Checks the settings look complete, failing as airpost-integrations does (`-32010` "Integration
 * failed: …"); the real test calls the provider.
 */
function mockIntegrationTest(p: IntegrationParams): IntegrationTestReport {
  const c = p?.config;
  const failed = (why: string) =>
    new RpcError({ code: RpcErrorCode.IntegrationFailed, message: `Integration failed: ${why}` });
  if (!c || !['libretranslate', 'deepl', 'languagetool', 'peopledatalabs'].includes(c.provider)) {
    throw invalidParams('config needs a provider');
  }
  // Required by the engine's config types: a missing one doesn't deserialize.
  if ((c.provider === 'deepl' || c.provider === 'peopledatalabs') && !c.apiKey)
    throw invalidParams('missing field `apiKey`');
  if (c.provider === 'libretranslate' && !c.url) throw invalidParams('missing field `url`');
  if (c.url) {
    try {
      const u = new URL(c.url);
      if (u.protocol !== 'http:' && u.protocol !== 'https:') throw new Error();
      return { host: u.host };
    } catch {
      throw failed('the server URL is not a valid http(s) URL');
    }
  }
  return { host: DEFAULT_HOSTS[c.provider]! };
}

function notImplemented(method: string): never {
  // The engine serves translate, grammar and enrich (DCP stage 5); no web view calls them
  // yet, so the mock has nothing to answer with.
  throw new RpcError({ code: RpcErrorCode.MethodNotFound, message: `${method} is not available` });
}

function invalidParams(message: string): RpcError {
  return new RpcError({ code: RpcErrorCode.InvalidParams, message: `invalid params: ${message}` });
}
