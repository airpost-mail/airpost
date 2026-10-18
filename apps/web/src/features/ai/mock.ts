// The mock engine's `ai.*` (docs/ai-plan.md sections 3 and 4) over its fixture store, with
// a pretend local model: canned text built from the thread, streamed as `ai.delta`, and a
// keyword triage. It checks what the engine checks, in the same order, and plays a relayed
// connection, so `config` and `settings` are refused. Imports engine modules directly, not
// '@/engine', because the engine's index loads the mock.
import { compareMessages, nowSeconds, threadHasRole } from '@/engine/model';
import type { MockStore } from '@/engine/mock/store';
import {
  RpcError,
  RpcErrorCode,
  type AiUsageRow,
  type EngineNotification,
  type RpcMethodName,
  type RpcMethods,
  type RpcResult,
} from '@/engine/rpc';
import { syncbackMetadata } from '@/engine/tasks';
import type { Delta, MessageJSON, TaskJSON, ThreadJSON } from '@/engine/types';
import {
  hostOf,
  isLocal,
  keyRequired,
  readAiSettings,
  type AiFeature,
  type AiProvider,
  type AiSettings,
} from './config';
import { PLUGIN_ID, answered, triageOf, type TriageCategory, type TriageValue } from './triage';

/** What the mock engine lends the AI methods. */
export interface MockAiHost {
  store: MockStore;
  /** The engine's settings store. */
  settings: Map<string, unknown>;
  /** To this connection only, without `subscribe` (as `ai.delta` goes). */
  send(n: EngineNotification): void;
  queueTask(accountId: string, task: TaskJSON): void;
  requireAccount(accountId: string | undefined): string;
  /** The mock's latency, scaled. */
  wait(scale?: number): Promise<void>;
}

type AiMethod = Extract<RpcMethodName, `ai.${string}`>;
export type MockAiHandlers = {
  [M in AiMethod]: (p: RpcMethods[M]['params']) => RpcResult<M> | Promise<RpcResult<M>>;
};

/** ms-ai PROMPT_VERSION as of AI4. */
const PROMPT_VERSION = 2;
const TRIAGE_RUN_MAX = 200;
const READ_MAX_LIMIT = 500;

export const DEMO_AI_SETTINGS = (accountIds: string[]): Record<string, unknown> => ({
  'core.ai.providers': [{ id: 'ollama', kind: 'ollama', model: 'llama3.2' }],
  'core.ai.features': {
    summary: { enabled: true, provider: 'ollama' },
    reply: { enabled: true, provider: 'ollama' },
    triage: { enabled: true, provider: 'ollama' },
  },
  'core.ai.accounts': accountIds,
  'core.ai.includeAddresses': false,
});

function failed(key: string, message: string): RpcError {
  return new RpcError({
    code: RpcErrorCode.AiFailed,
    message: 'AI request failed',
    data: { key, message, retryable: false },
  });
}

function invalid(message: string): RpcError {
  return new RpcError({ code: RpcErrorCode.InvalidParams, message: `invalid params: ${message}` });
}

const firstName = (m: MessageJSON | undefined) => {
  const c = m?.from[0];
  return (c?.name?.trim() || c?.email || 'there').split(/[\s@]/)[0];
};

function plain(m: MessageJSON, store: MockStore): string {
  const body = store.body(m.id) ?? m.snippet ?? '';
  return body
    .replace(/<(style|script)[\s\S]*?<\/\1>/gi, ' ')
    .replace(/<[^>]+>/g, ' ')
    .replace(/&nbsp;/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

/** Pieces of a few words each, as a model streams. */
function chunks(text: string): string[] {
  return (
    text.match(/\S+\s*/g)?.reduce<string[]>((out, word, i) => {
      if (i % 3 === 0) out.push(word);
      else out[out.length - 1] += word;
      return out;
    }, []) ?? []
  );
}

export class MockAi {
  private readonly streams = new Map<string, { cancelled: boolean }>();
  private readonly jobs = new Map<string, { cancelled: boolean }>();
  private readonly summaries = new Map<string, string>();
  private readonly usage = new Map<string, AiUsageRow>();
  private nextJob = 1;

  constructor(private readonly host: MockAiHost) {}

  /** A local Ollama set up for every account, and the inboxes triaged. For `npm run dev`. */
  seedDemo() {
    const { store, settings } = this.host;
    for (const [k, v] of Object.entries(DEMO_AI_SETTINGS([...store.accounts.keys()])))
      settings.set(k, v);
    const s = this.settings();
    for (const t of store.threads.values()) {
      if (!threadHasRole(t, 'inbox')) continue;
      const value = this.triage(t, s.providers[0]);
      if (!value) continue;
      t.metadata = [...(t.metadata ?? []), { pluginId: PLUGIN_ID, v: 1, value }];
    }
  }

  /** New mail: triage each new inbox thread of an opted-in account, as the worker does. */
  newMail(deltas: Delta[]) {
    for (const d of deltas) {
      if (d.modelClass !== 'Thread' || d.type !== 'persist') continue;
      for (const t of d.modelJSONs as ThreadJSON[]) {
        void this.host.wait(2).then(() => {
          const s = this.settings();
          const provider = this.provider(s, 'triage');
          const now = this.host.store.threads.get(t.id);
          if (!now || !provider || !this.triageOn(s, now.aid) || !threadHasRole(now, 'inbox'))
            return;
          this.fileTriage(now, provider);
        });
      }
    }
  }

  handlers(): MockAiHandlers {
    return {
      'ai.models': (p) => {
        this.relayed(p);
        const provider = this.named(p.provider);
        return [{ id: provider.model, name: provider.model }];
      },
      'ai.test': (p) => {
        this.relayed(p);
        const provider = this.named(p.provider);
        return {
          host: hostOf(provider),
          model: provider.model,
          local: isLocal(provider),
          latencyMs: 42,
        };
      },
      'ai.summarize': async (p) => {
        const { provider, thread, messages } = this.prepare('summary', p);
        const key = `${thread.id}:${messages[messages.length - 1].id}`;
        const known = this.summaries.get(key);
        if (known && !p.refresh) return { text: known, model: provider.model, cached: true };
        const text = await this.stream(p.streamId, this.summaryText(thread, messages));
        this.summaries.set(key, text);
        const usage = this.count(provider, 'summary', messages, text);
        return { text, model: provider.model, cached: false, usage };
      },
      'ai.suggestReplies': async (p) => {
        const { provider, messages } = this.prepare('reply', p);
        const count = Math.min(Math.max(p.count ?? 3, 1), 3);
        await this.stream(p.streamId, '', { deltas: false });
        const name = firstName(messages[messages.length - 1]);
        const all = [
          { label: 'Thanks', text: `Thank ${name} and say you'll read it properly today.` },
          { label: 'Yes', text: 'Say yes and confirm the details.' },
          { label: 'Later', text: 'Say you need a few days and will come back by Friday.' },
        ];
        this.count(provider, 'reply', messages, all.map((s) => s.text).join('\n'));
        return all.slice(0, count);
      },
      'ai.draftReply': async (p) => {
        const { provider, thread, messages } = this.prepare('reply', p);
        const own = this.host.store.ownEmails(thread.aid);
        const inbound = [...messages]
          .reverse()
          .find((m) => !own.has(m.from[0]?.email.toLowerCase() ?? ''));
        const owner = this.host.store.account(thread.aid)?.name?.split(' ')[0] ?? '';
        const ask = (p.instruction ?? p.suggestion ?? 'Thank them for the message.').trim();
        const text = [
          `Hi ${firstName(inbound)},`,
          '',
          `Thanks for your note about ${thread.subject || 'this'}. ${ask.charAt(0).toUpperCase()}${ask.slice(1)}${/[.!?]$/.test(ask) ? '' : '.'}`,
          '',
          `Best,`,
          owner,
        ].join('\n');
        const out = await this.stream(p.streamId, text);
        return {
          text: out,
          model: provider.model,
          usage: this.count(provider, 'reply', messages, out),
        };
      },
      'ai.cancel': (p) => {
        if (p.streamId) {
          const s = this.streams.get(p.streamId);
          if (s) s.cancelled = true;
        }
        if (p.jobId) {
          const j = this.jobs.get(p.jobId);
          if (j) j.cancelled = true;
        }
        return {};
      },
      'ai.usage': (p) => {
        const days = p?.days ?? 30;
        const since = new Date(Date.now() - days * 86_400_000).toISOString().slice(0, 10);
        return [...this.usage.values()]
          .filter((r) => r.day >= since)
          .sort((a, b) => (a.day < b.day ? 1 : a.day > b.day ? -1 : 0));
      },
      'ai.triage.run': (p) => {
        this.relayed(p);
        const accountId = this.host.requireAccount(p.accountId);
        const s = this.settings();
        if (!this.triageOn(s, accountId))
          throw failed('not-enabled', 'AI triage is off for this account');
        const provider = p.provider ? this.named(p.provider) : this.provider(s, 'triage');
        if (!provider) throw failed('not-configured', 'the model provider is not configured');
        const { store } = this.host;
        let threads: ThreadJSON[];
        if (p.threadIds) {
          threads = p.threadIds
            .map((id) => store.threads.get(id))
            .filter((t): t is ThreadJSON => !!t && t.aid === accountId);
        } else {
          const since = nowSeconds() - (p.sinceDays ?? 7) * 86_400;
          threads = [...store.threads.values()]
            .filter((t) => t.aid === accountId && threadHasRole(t, 'inbox') && t.lmrt >= since)
            .sort((a, b) => b.lmrt - a.lmrt);
        }
        threads = threads.slice(0, TRIAGE_RUN_MAX);
        const jobId = `triage-${this.nextJob++}`;
        const job = { cancelled: false };
        this.jobs.set(jobId, job);
        void (async () => {
          for (let i = 0; i < threads.length; i++) {
            await this.host.wait(1);
            if (job.cancelled) {
              this.progress({ jobId, done: i, total: threads.length, stopped: true });
              break;
            }
            this.fileTriage(threads[i], provider);
            this.progress({ jobId, done: i + 1, total: threads.length });
          }
          this.jobs.delete(jobId);
        })();
        return { jobId, queued: threads.length };
      },
      'ai.triage.list': (p) => {
        if (!p || typeof p.limit !== 'number') throw invalid('missing field `limit`');
        const s = this.settings();
        const asked = p.accountIds ?? [...this.host.store.accounts.keys()];
        for (const a of asked) this.host.requireAccount(a);
        const accounts = asked.filter((a) => s.accounts.includes(a));
        const category = p.category as TriageCategory | undefined;
        const matches = [...this.host.store.threads.values()]
          .filter((t) => {
            if (!accounts.includes(t.aid) || !threadHasRole(t, 'inbox')) return false;
            const v = triageOf(t);
            if (!v || !category) return !!v;
            if (category !== 'needs-reply') return v.category === category;
            return (v.category === 'needs-reply' || !!v.needsReply) && !answered(v, t);
          })
          .sort((a, b) => b.lmrt - a.lmrt || (a.id < b.id ? 1 : -1));
        const offset = p.offset ?? 0;
        const limit = Math.min(Math.max(p.limit, 1), READ_MAX_LIMIT);
        return {
          threadIds: matches.slice(offset, offset + limit).map((t) => t.id),
          total: matches.length,
        };
      },
    };
  }

  // ------------------------------------------------------------------ internals

  private settings(): AiSettings {
    return readAiSettings(Object.fromEntries(this.host.settings));
  }

  private triageOn(s: AiSettings, accountId: string) {
    return s.features.triage.enabled && s.accounts.includes(accountId);
  }

  /** The gateway refuses these on a relayed connection, admin or not. */
  private relayed(p: unknown) {
    const params = (p ?? {}) as Record<string, unknown>;
    for (const k of ['config', 'settings']) {
      if (params[k] !== undefined && params[k] !== null) {
        throw new RpcError({
          code: RpcErrorCode.NotAllowedRelayed,
          message: `${k} is not allowed on a relayed connection`,
        });
      }
    }
  }

  /** The provider ready to call, or why not, as the engine's resolve_provider. */
  private usable(provider: AiProvider | null): AiProvider {
    if (!provider || !provider.model.trim())
      throw failed('not-configured', 'the model provider is not configured');
    // The engine holds no keys yet, so a provider that needs one can't run from the store.
    if (keyRequired(provider)) throw failed('not-configured', 'the API key is not configured');
    return provider;
  }

  private named(id: string | undefined): AiProvider {
    const provider = this.settings().providers.find((p) => p.id === id) ?? null;
    return this.usable(provider);
  }

  private provider(s: AiSettings, feature: AiFeature): AiProvider | null {
    const id = s.features[feature].provider;
    return s.providers.find((p) => p.id === id) ?? null;
  }

  /** The checks a thread call passes before any mail is read, then the mail it may send. */
  private prepare(
    feature: AiFeature,
    p: { accountId: string; threadId: string; streamId?: string; provider?: string },
  ) {
    this.relayed(p);
    const accountId = this.host.requireAccount(p.accountId);
    const s = this.settings();
    if (!s.accounts.includes(accountId) || !s.features[feature].enabled)
      throw failed('not-enabled', `AI ${feature} is off for this account`);
    const provider = p.provider ? this.named(p.provider) : this.usable(this.provider(s, feature));
    const { store } = this.host;
    const thread = store.threads.get(p.threadId);
    if (!thread || thread.aid !== accountId) {
      throw new RpcError({ code: RpcErrorCode.NotFound, message: `no thread ${p.threadId}` });
    }
    const hidden = (m: MessageJSON) =>
      Object.keys(m.folders ?? {}).every((id) => {
        const role = store.categories.get(id)?.role;
        return role === 'spam' || role === 'trash';
      });
    const messages = store
      .threadMessages(thread.id)
      .filter((m) => !m.draft && !hidden(m))
      .sort(compareMessages);
    if (messages.length === 0) throw failed('unsupported', 'nothing in this thread may be sent');
    if (p.streamId && this.streams.has(p.streamId))
      throw invalid(`stream ${p.streamId} is already running`);
    return { provider, thread, messages };
  }

  /** Streams `text` as `ai.delta` under `streamId` (or just waits), until cancelled. */
  private async stream(
    streamId: string | undefined,
    text: string,
    { deltas = true }: { deltas?: boolean } = {},
  ): Promise<string> {
    const state = { cancelled: false };
    if (streamId) this.streams.set(streamId, state);
    try {
      const pieces = deltas ? chunks(text) : [''];
      for (const piece of pieces) {
        await this.host.wait(0.4);
        if (state.cancelled) throw failed('cancelled', 'the request was cancelled');
        if (deltas && streamId) {
          this.host.send({ method: 'ai.delta', params: { streamId, text: piece } });
        }
      }
      return text;
    } finally {
      if (streamId) this.streams.delete(streamId);
    }
  }

  private summaryText(thread: ThreadJSON, messages: MessageJSON[]): string {
    const { store } = this.host;
    const names = [...new Set(messages.map((m) => firstName(m)))];
    const last = messages[messages.length - 1];
    const latest = plain(last, store).slice(0, 180);
    const who =
      names.length > 2
        ? `${names.slice(0, -1).join(', ')} and ${names.at(-1)}`
        : names.join(' and ');
    return [
      `${who} ${messages.length > 1 ? `wrote ${messages.length} messages` : 'wrote'} about "${thread.subject || 'no subject'}".`,
      latest
        ? `The latest, from ${firstName(last)}: ${latest}${latest.length === 180 ? '…' : ''}`
        : '',
      /\?/.test(latest) ? 'There is an open question for you.' : 'Nothing seems to wait on you.',
    ]
      .filter(Boolean)
      .join('\n\n');
  }

  private count(provider: AiProvider, feature: string, messages: MessageJSON[], out: string) {
    const input = Math.ceil(messages.reduce((n, m) => n + plain(m, this.host.store).length, 0) / 4);
    const output = Math.ceil(out.length / 4);
    const day = new Date().toISOString().slice(0, 10);
    const key = `${day}|${provider.id}|${feature}`;
    const row = this.usage.get(key) ?? {
      day,
      provider: provider.id,
      feature,
      inputTokens: 0,
      outputTokens: 0,
      requests: 0,
    };
    row.inputTokens += input;
    row.outputTokens += output;
    row.requests += 1;
    this.usage.set(key, row);
    return { inputTokens: input, outputTokens: output };
  }

  private progress(params: { jobId: string; done: number; total: number; stopped?: boolean }) {
    this.host.send({ method: 'ai.triage.progress', params });
  }

  /** The pretend model's answer for the thread's latest inbound message. */
  private triage(t: ThreadJSON, provider: AiProvider | undefined): TriageValue | null {
    const { store } = this.host;
    const own = store.ownEmails(t.aid);
    const msgs = store.threadMessages(t.id).filter((m) => !m.draft);
    const inbound = [...msgs].reverse().find((m) => !own.has(m.from[0]?.email.toLowerCase() ?? ''));
    if (!inbound) return null;
    const from = inbound.from[0]?.email.toLowerCase() ?? '';
    const text = `${t.subject} ${plain(inbound, store)}`;
    let category: TriageCategory = 'other';
    let reason = 'Nothing here needs you.';
    if (inbound.hListUnsub) {
      category = 'newsletter';
      reason = 'A mailing list.';
    } else if (/no-?reply|notifications?@|alerts?@|billing@/.test(from)) {
      category = 'notification';
      reason = 'An automated message.';
    } else if (/\b(sale|offer|deal|% off)\b/i.test(text)) {
      category = 'promotion';
      reason = 'An offer.';
    } else if (/\?|\b(can you|could you|let me know|please confirm)\b/i.test(text)) {
      category = 'needs-reply';
      reason = `${firstName(inbound)} asked you something.`;
    } else if (t.starred > 0) {
      category = 'important';
      reason = 'You starred it.';
    }
    return {
      category,
      needsReply: category === 'needs-reply',
      reason,
      messageId: inbound.id,
      model: provider?.model ?? 'llama3.2',
      v: PROMPT_VERSION,
      at: nowSeconds(),
    };
  }

  /** Files a triage value through the local metadata path, unless the user filed one. */
  private fileTriage(t: ThreadJSON, provider: AiProvider) {
    const before = triageOf(t);
    if (before?.source === 'user') return;
    const value = this.triage(t, provider);
    if (!value || (before?.messageId === value.messageId && before?.v === PROMPT_VERSION)) return;
    this.host.queueTask(
      t.aid,
      syncbackMetadata({
        accountId: t.aid,
        pluginId: PLUGIN_ID,
        modelId: t.id,
        modelClassName: 'thread',
        value,
      }),
    );
  }
}
