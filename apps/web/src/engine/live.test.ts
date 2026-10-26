import { QueryClient } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { threadListQuery, type ThreadList } from './hooks';
import { engineKeys, EVERY_ACCOUNT, type ThreadQuery } from './keys';
import { liveQueryOptions, setLiveWindow, type LiveSnapshot } from './live';
import { MockEngine } from './mock/engine';
import { MOCK_ACCOUNT_IDS } from './mock/fixtures';
import * as tasks from './mock/tasks';
import type { ThreadJSON } from './types';

const G = MOCK_ACCOUNT_IDS.gmail;
const starredQuery: ThreadQuery = { accountId: EVERY_ACCOUNT, starred: true };

function starredList(engine: MockEngine, limit = 50) {
  return liveQueryOptions(engine, {
    key: engineKeys.threadList(starredQuery),
    query: () => threadListQuery(starredQuery),
    window: { offset: 0, limit },
    select: (s: ThreadList) => s,
  });
}

describe('live query options', () => {
  let engine: MockEngine;
  let qc: QueryClient;
  beforeEach(async () => {
    engine = new MockEngine({ latency: 0 });
    qc = new QueryClient();
    await engine.connect();
  });
  afterEach(() => {
    qc.clear();
    engine.close();
  });

  const starredInStore = () =>
    [...engine.store.threads.values()].filter((t) => t.inAllMail && t.starred > 0);
  const cached = () => qc.getQueryData<ThreadList>(engineKeys.threadList(starredQuery))!;

  it("reads the engine's window with each row's summary", async () => {
    const list = await qc.fetchQuery(starredList(engine));
    expect(list.total).toBe(starredInStore().length);
    expect(list.rows.map((t) => t.id).sort()).toEqual(
      starredInStore()
        .map((t) => t.id)
        .sort(),
    );
    const first = list.rows[0];
    const visible = engine.store.threadMessages(first.id).filter((m) => !m.hidden);
    expect(first.summary?.messageCount).toBe(visible.length);
    expect(first.summary?.senders.length).toBeGreaterThan(0);
  });

  it('keeps the cache in step with the engine', async () => {
    await qc.fetchQuery(starredList(engine));
    const unstarred = [...engine.store.threads.values()].find(
      (t) => t.aid === G && t.inAllMail && t.starred === 0,
    )!;
    await engine.call('task.queue', {
      accountId: G,
      task: tasks.changeStarred({ accountId: G, threadIds: [unstarred.id], starred: true }),
    });
    await vi.waitFor(() => expect(cached().rows.map((t) => t.id)).toContain(unstarred.id));
    expect(cached().total).toBe(starredInStore().length);
  });

  it('moves the window', async () => {
    const list = await qc.fetchQuery(starredList(engine, 1));
    expect(list.rows).toHaveLength(1);
    setLiveWindow(qc, engineKeys.threadList(starredQuery), { offset: 0, limit: 3 });
    await vi.waitFor(() => expect(cached().rows).toHaveLength(Math.min(3, list.total)));
  });

  it('ends the subscription when the cache drops the query', async () => {
    await qc.fetchQuery(starredList(engine));
    expect(engine.live.size).toBe(1);
    qc.removeQueries({ queryKey: engineKeys.threadList(starredQuery) });
    expect(engine.live.size).toBe(0);
  });

  it('subscribes again after a reconnect', async () => {
    await qc.fetchQuery(starredList(engine));
    engine.close();
    await engine.connect();
    const starred = starredInStore()[0];
    await engine.call('task.queue', {
      accountId: starred.aid,
      task: tasks.changeStarred({
        accountId: starred.aid,
        threadIds: [starred.id],
        starred: false,
      }),
    });
    await vi.waitFor(() => expect(cached().rows.map((t) => t.id)).not.toContain(starred.id));
  });

  it('rejects with the refusal of a query the engine refuses', async () => {
    const refused = liveQueryOptions(engine, {
      key: ['engine', 'test', 'refused'],
      query: () => ({ class: 'Thread', where: [{ attr: 'nope', op: '=', value: 1 }] }),
      select: (s: LiveSnapshot<ThreadJSON>) => s,
    });
    await expect(qc.fetchQuery(refused)).rejects.toMatchObject({ code: -32602 });
  });
});
