import { describe, expect, it } from 'vitest';
import {
  MOCK_ACCOUNT_IDS,
  MockEngine,
  RpcError,
  RpcErrorCode,
  threadHasRole,
  type EngineNotification,
  type ThreadJSON,
} from '@/engine';
import { DEMO_AI_SETTINGS } from '../mock';
import { PLUGIN_ID, correctionTask, triageOf } from '../triage';

const G = MOCK_ACCOUNT_IDS.gmail;
const I = MOCK_ACCOUNT_IDS.imap;

async function engine(opts: { aiDemo?: boolean; latency?: number } = {}) {
  const e = new MockEngine({ latency: opts.latency ?? 0, aiDemo: opts.aiDemo ?? true });
  await e.connect();
  const notes: EngineNotification[] = [];
  e.onNotification((n) => notes.push(n));
  return { e, notes };
}

/** A thread of the account with at least `min` messages. */
function threadOf(e: MockEngine, accountId: string, min = 2): ThreadJSON {
  const t = [...e.store.threads.values()].find(
    (t) =>
      t.aid === accountId && e.store.threadMessages(t.id).filter((m) => !m.draft).length >= min,
  );
  if (!t) throw new Error('no fixture thread');
  return t;
}

async function failure(p: Promise<unknown>): Promise<RpcError> {
  try {
    await p;
  } catch (err) {
    if (err instanceof RpcError) return err;
    throw err;
  }
  throw new Error('expected the call to fail');
}

describe('mock ai.* checks', () => {
  it('is not-enabled until the account is opted in and the feature on', async () => {
    const { e } = await engine({ aiDemo: false });
    const t = threadOf(e, G);
    const err = await failure(e.call('ai.summarize', { accountId: G, threadId: t.id }));
    expect(err.code).toBe(RpcErrorCode.AiFailed);
    expect(err.data).toMatchObject({ key: 'not-enabled' });

    e.changeSettings(DEMO_AI_SETTINGS([I]));
    const other = await failure(e.call('ai.summarize', { accountId: G, threadId: t.id }));
    expect(other.data).toMatchObject({ key: 'not-enabled' });
  });

  it('refuses config and settings, as the relay does', async () => {
    const { e } = await engine();
    const t = threadOf(e, G);
    for (const extra of [{ config: { kind: 'ollama' } }, { settings: { accounts: [G] } }]) {
      const err = await failure(
        e.call('ai.summarize', { accountId: G, threadId: t.id, ...extra } as never),
      );
      expect(err.code).toBe(RpcErrorCode.NotAllowedRelayed);
    }
    const set = await failure(e.call('settings.set', { values: { 'core.ai.accounts': [G] } }));
    expect(set.code).toBe(RpcErrorCode.NotAllowedRelayed);
  });

  it('is not-configured for a stored provider that needs a key', async () => {
    const { e } = await engine();
    e.changeSettings({
      'core.ai.providers': [{ id: 'claude', kind: 'anthropic', model: 'claude-x' }],
      'core.ai.features': { summary: { enabled: true, provider: 'claude' } },
    });
    const t = threadOf(e, G);
    const err = await failure(e.call('ai.summarize', { accountId: G, threadId: t.id }));
    expect(err.data).toMatchObject({ key: 'not-configured' });
  });

  it('answers unknown accounts and threads with the engine codes', async () => {
    const { e } = await engine();
    expect((await failure(e.call('ai.summarize', { accountId: 'x', threadId: 'y' }))).code).toBe(
      RpcErrorCode.UnknownAccount,
    );
    expect((await failure(e.call('ai.summarize', { accountId: G, threadId: 'nope' }))).code).toBe(
      RpcErrorCode.NotFound,
    );
  });
});

describe('mock ai.summarize', () => {
  it('streams ai.delta in order to this connection, then the full text', async () => {
    const { e, notes } = await engine();
    const t = threadOf(e, G);
    const r = await e.call('ai.summarize', { accountId: G, threadId: t.id, streamId: 's1' });
    const deltas = notes.filter((n) => n.method === 'ai.delta');
    expect(deltas.length).toBeGreaterThan(1);
    expect(deltas.map((n) => (n.params as { text: string }).text).join('')).toBe(r.text);
    expect(r).toMatchObject({ model: 'llama3.2', cached: false });

    const again = await e.call('ai.summarize', { accountId: G, threadId: t.id, streamId: 's2' });
    expect(again).toMatchObject({ text: r.text, cached: true });
    const fresh = await e.call('ai.summarize', {
      accountId: G,
      threadId: t.id,
      streamId: 's3',
      refresh: true,
    });
    expect(fresh.cached).toBe(false);
  });

  it('stops on ai.cancel with a cancelled error', async () => {
    const { e, notes } = await engine({ latency: 5 });
    const t = threadOf(e, G);
    const p = e.call('ai.summarize', { accountId: G, threadId: t.id, streamId: 'c1' });
    await new Promise<void>((resolve) => {
      const off = e.onNotification((n) => {
        if (n.method === 'ai.delta') {
          off();
          resolve();
        }
      });
    });
    await e.call('ai.cancel', { streamId: 'c1' });
    const err = await failure(p);
    expect(err.data).toMatchObject({ key: 'cancelled' });
    expect(notes.filter((n) => n.method === 'ai.delta').length).toBeLessThan(10);
  });

  it('counts usage per provider and feature', async () => {
    const { e } = await engine();
    const t = threadOf(e, G);
    await e.call('ai.summarize', { accountId: G, threadId: t.id });
    const rows = await e.call('ai.usage', {});
    expect(rows).toEqual([
      expect.objectContaining({ provider: 'ollama', feature: 'summary', requests: 1 }),
    ]);
  });
});

describe('mock replies', () => {
  it('suggests at most three and drafts from a pick', async () => {
    const { e, notes } = await engine();
    const t = threadOf(e, G);
    const list = await e.call('ai.suggestReplies', { accountId: G, threadId: t.id, count: 5 });
    expect(list).toHaveLength(3);
    expect(notes.some((n) => n.method === 'ai.delta')).toBe(false);
    const draft = await e.call('ai.draftReply', {
      accountId: G,
      threadId: t.id,
      instruction: "say I'll call Tuesday",
      streamId: 'd1',
    });
    expect(draft.text).toContain("Say I'll call Tuesday.");
  });
});

describe('mock triage', () => {
  it('lists needs-reply inbox threads newest first, and drops answered ones', async () => {
    const { e } = await engine();
    const all = await e.call('ai.triage.list', { category: 'needs-reply', limit: 500 });
    expect(all.total).toBeGreaterThan(0);
    const threads = all.threadIds.map((id) => e.store.threads.get(id)!);
    for (const t of threads) {
      expect(threadHasRole(t, 'inbox')).toBe(true);
      expect(triageOf(t)?.needsReply).toBe(true);
    }
    expect(threads.map((t) => t.lmrt)).toEqual(
      [...threads.map((t) => t.lmrt)].sort((a, b) => b - a),
    );

    const t = threads[0];
    t.lmst = t.lmrt + 1;
    const after = await e.call('ai.triage.list', { category: 'needs-reply', limit: 500 });
    expect(after.threadIds).not.toContain(t.id);
  });

  it('lists only opted-in accounts', async () => {
    const { e } = await engine();
    e.changeSettings({ 'core.ai.accounts': [I] });
    const r = await e.call('ai.triage.list', { limit: 500 });
    expect(r.threadIds.every((id) => e.store.threads.get(id)!.aid === I)).toBe(true);
  });

  it('runs a job with progress to the caller and keeps a user value', async () => {
    const { e, notes } = await engine({ aiDemo: false });
    e.changeSettings(DEMO_AI_SETTINGS([G]));
    const inbox = [...e.store.threads.values()].filter(
      (t) => t.aid === G && threadHasRole(t, 'inbox'),
    );
    const mine = inbox[0];
    await e.call('task.queue', { accountId: G, task: correctionTask(mine, 'important') });
    const ids = inbox.slice(0, 3).map((t) => t.id);
    const { jobId, queued } = await e.call('ai.triage.run', { accountId: G, threadIds: ids });
    expect(queued).toBe(3);
    await until(() =>
      notes.some(
        (n) => n.method === 'ai.triage.progress' && (n.params as { done: number }).done === 3,
      ),
    );
    const progress = notes.filter((n) => n.method === 'ai.triage.progress');
    expect(progress.map((n) => n.params)).toEqual([
      { jobId, done: 1, total: 3 },
      { jobId, done: 2, total: 3 },
      { jobId, done: 3, total: 3 },
    ]);
    await until(() => ids.slice(1).every((id) => triageOf(e.store.threads.get(id))));
    expect(triageOf(e.store.threads.get(mine.id))).toMatchObject({
      category: 'important',
      source: 'user',
    });
    expect(e.store.threads.get(ids[1])!.metadata?.some((m) => m.pluginId === PLUGIN_ID)).toBe(true);
  });

  it('ends a cancelled job with a stopped progress', async () => {
    const { e, notes } = await engine({ aiDemo: false, latency: 10 });
    e.changeSettings(DEMO_AI_SETTINGS([G]));
    const ids = [...e.store.threads.values()]
      .filter((t) => t.aid === G && threadHasRole(t, 'inbox'))
      .slice(0, 3)
      .map((t) => t.id);
    const { jobId } = await e.call('ai.triage.run', { accountId: G, threadIds: ids });
    await e.call('ai.cancel', { jobId });
    await until(() =>
      notes.some((n) => n.method === 'ai.triage.progress' && n.params.stopped === true),
    );
    const progress = notes.flatMap((n) => (n.method === 'ai.triage.progress' ? [n.params] : []));
    const last = progress[progress.length - 1];
    expect(last).toMatchObject({ jobId, total: 3, stopped: true });
    expect(last.done).toBeLessThan(3);
  });
});

async function until(ok: () => boolean, ms = 2000) {
  const start = Date.now();
  while (!ok()) {
    if (Date.now() - start > ms) throw new Error('timed out');
    await new Promise((r) => setTimeout(r, 5));
  }
}
