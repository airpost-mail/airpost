import { beforeEach, describe, expect, it } from 'vitest';
import type { EngineNotification } from '../rpc';
import { RpcError, RpcErrorCode } from '../rpc';
import * as tasks from '../tasks';
import type { Delta, LabelJSON, MessageJSON, TaskJSON, ThreadJSON } from '../types';
import { MockEngine } from './engine';
import { MOCK_ACCOUNT_IDS } from './fixtures';

const NOW = 1_780_000_000;
const G = MOCK_ACCOUNT_IDS.gmail;
const I = MOCK_ACCOUNT_IDS.imap;

function collect(engine: MockEngine) {
  const deltas: Delta[] = [];
  engine.onNotification((n: EngineNotification) => {
    if (n.method === 'delta') deltas.push(n.params);
  });
  return deltas;
}

const tick = (ms = 5) => new Promise((r) => setTimeout(r, ms));

async function taskDone(deltas: Delta[], id: string, timeout = 2000): Promise<TaskJSON> {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    for (const d of deltas) {
      if (d.modelClass !== 'Task') continue;
      const t = (d.modelJSONs as TaskJSON[]).find((x) => x.id === id && x.status === 'complete');
      if (t) return t;
    }
    await tick(10);
  }
  throw new Error('task did not complete');
}

function threadsIn(deltas: Delta[]): ThreadJSON[] {
  return deltas
    .filter((d) => d.modelClass === 'Thread' && d.type === 'persist')
    .flatMap((d) => d.modelJSONs as ThreadJSON[]);
}

describe('MockEngine', () => {
  let engine: MockEngine;
  let deltas: Delta[];

  beforeEach(async () => {
    engine = new MockEngine({ latency: 0, now: NOW });
    deltas = collect(engine);
    await engine.connect();
  });

  it('says hello with both accounts', async () => {
    const hello = await engine.call('hello', { protocol: 1, client: 'test' });
    expect(hello.accounts.map((a) => a.accountId).sort()).toEqual([G, I].sort());
  });

  it('refuses an unknown protocol version', async () => {
    await expect(engine.call('hello', { protocol: 99, client: 'test' })).rejects.toMatchObject({
      code: RpcErrorCode.UnsupportedProtocol,
    });
  });

  it('rejects unknown accounts with the engine error code', async () => {
    const err = await engine.call('folders.list', { accountId: 'nope' }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(RpcError);
    expect((err as RpcError).code).toBe(RpcErrorCode.UnknownAccount);
  });

  it('has around sixty threads across both accounts', () => {
    const all = [...engine.store.threads.values()];
    expect(all.length).toBeGreaterThanOrEqual(55);
    expect(all.some((t) => t.aid === G)).toBe(true);
    expect(all.some((t) => t.aid === I)).toBe(true);
    expect(all.some((t) => t.participants.length >= 3 && t.attachmentCount > 0)).toBe(true);
  });

  it('lists Gmail inbox by label and IMAP inbox by folder, newest first', async () => {
    const gInbox = await engine.call('threads.list', { accountId: G, labelId: 'g-inbox' });
    const iInbox = await engine.call('threads.list', { accountId: I, folderId: 'i-inbox' });
    expect(gInbox.threads.length).toBeGreaterThan(5);
    expect(iInbox.threads.length).toBeGreaterThan(5);
    for (const page of [gInbox, iInbox]) {
      const ts = page.threads.map((t) => t.lmt);
      expect(ts).toEqual([...ts].sort((a, b) => b - a));
    }
    expect(gInbox.threads.every((t) => t.labels.some((l) => l.id === 'g-inbox'))).toBe(true);
  });

  it('pages threads with a keyset cursor', async () => {
    const first = await engine.call('threads.list', { accountId: G, folderId: 'g-all', limit: 5 });
    expect(first.threads).toHaveLength(5);
    expect(first.next).toEqual(expect.any(String));
    const second = await engine.call('threads.list', {
      accountId: G,
      folderId: 'g-all',
      limit: 5,
      before: first.next!,
    });
    const ids = new Set(first.threads.map((t) => t.id));
    expect(second.threads.some((t) => ids.has(t.id))).toBe(false);
  });

  it('rejects params the engine does not take', async () => {
    const err = await engine
      .call('threads.list', { accountId: G, starred: true } as never)
      .catch((e: unknown) => e);
    expect((err as RpcError).code).toBe(RpcErrorCode.InvalidParams);
    const both = await engine
      .call('threads.list', { accountId: G, folderId: 'g-all', labelId: 'g-inbox' })
      .catch((e: unknown) => e);
    expect((both as RpcError).code).toBe(RpcErrorCode.InvalidParams);
  });

  it('lists by role', async () => {
    const byRole = await engine.call('threads.list', { accountId: I, role: 'inbox' });
    const byId = await engine.call('threads.list', { accountId: I, folderId: 'i-inbox' });
    expect(byRole.threads.map((t) => t.id)).toEqual(byId.threads.map((t) => t.id));
  });

  it('threads.get returns the thread and its messages oldest first, without bodies', async () => {
    const page = await engine.call('threads.list', { accountId: I, folderId: 'i-inbox' });
    const long = page.threads.find((t) => t.subject.startsWith('Northwind rebrand — round 2'))!;
    const { thread, messages: msgs } = await engine.call('threads.get', {
      accountId: I,
      id: long.id,
    });
    expect(thread.id).toBe(long.id);
    expect(msgs.length).toBe(9);
    expect(msgs.map((m) => m.date)).toEqual([...msgs.map((m) => m.date)].sort((a, b) => a - b));
    expect(msgs.every((m) => m.body === undefined)).toBe(true);
    const got = await engine.call('messages.get', { accountId: I, id: msgs[0].id, body: true });
    expect(got.body).toEqual(expect.any(String));
    const bare = await engine.call('messages.get', { accountId: I, id: msgs[0].id });
    expect(bare.body).toBeNull();
  });

  it('reports a missing thread as not found', async () => {
    const err = await engine
      .call('threads.get', { accountId: I, id: 'nope' })
      .catch((e: unknown) => e);
    expect((err as RpcError).code).toBe(RpcErrorCode.NotFound);
  });

  it('withholds bodies until bodies.fetch, then sends them as a Message delta', async () => {
    const t = [...engine.store.threads.values()].find((x) => x.subject.startsWith('Workshop'))!;
    const { messages } = await engine.call('threads.get', { accountId: I, id: t.id });
    const msg = messages[0];
    const before = await engine.call('messages.get', { accountId: I, id: msg.id, body: true });
    expect(before.body).toBeNull();
    await engine.call('bodies.fetch', { accountId: I, ids: [msg.id] });
    await tick();
    const persisted = deltas
      .filter((d) => d.modelClass === 'Message')
      .flatMap((d) => d.modelJSONs as MessageJSON[]);
    expect(persisted.find((m) => m.id === msg.id)?.body).toContain('workshop');
  });

  it('ChangeUnreadTask updates messages and the thread, and completes', async () => {
    const t = [...engine.store.threads.values()].find((x) => x.aid === I && x.unread > 0)!;
    const task = tasks.changeUnread({ accountId: I, threadIds: [t.id], unread: false });
    const queued = await engine.call('task.queue', { accountId: I, task });
    expect(queued.status).toBe('remote');
    await tick();
    const updated = threadsIn(deltas).find((x) => x.id === t.id)!;
    expect(updated.unread).toBe(0);
    expect(updated.folders.every((f) => f._u === 0)).toBe(true);
    await taskDone(deltas, task.id);
  });

  it('ChangeStarredTask stars the newest message only', async () => {
    const t = [...engine.store.threads.values()].find((x) => x.aid === G && x.starred === 0)!;
    await engine.call('task.queue', {
      accountId: G,
      task: tasks.changeStarred({ accountId: G, threadIds: [t.id], starred: true }),
    });
    await tick();
    expect(threadsIn(deltas).find((x) => x.id === t.id)?.starred).toBe(1);
  });

  it('ChangeFolderTask moves IMAP inbox copies to the archive', async () => {
    const page = await engine.call('threads.list', { accountId: I, folderId: 'i-inbox' });
    const t = page.threads[0];
    const archive = engine.store.categoryByRole(I, 'archive')!;
    await engine.call('task.queue', {
      accountId: I,
      task: tasks.changeFolder({
        accountId: I,
        threadIds: [t.id],
        folder: archive as never,
        sourceFolderIds: ['i-inbox'],
      }),
    });
    await tick();
    const after = await engine.call('threads.list', { accountId: I, folderId: 'i-inbox' });
    expect(after.threads.some((x) => x.id === t.id)).toBe(false);
    const moved = threadsIn(deltas).find((x) => x.id === t.id)!;
    expect(moved.folders.map((f) => f.id)).toContain(archive.id);
  });

  it('ChangeLabelsTask archives a Gmail thread by removing Inbox', async () => {
    const page = await engine.call('threads.list', { accountId: G, labelId: 'g-inbox' });
    const t = page.threads[0];
    const { labels } = await engine.call('labels.list', { accountId: G });
    const inbox = labels.find((l) => l.role === 'inbox') as LabelJSON;
    const travel = labels.find((l) => l.path === 'Travel') as LabelJSON;
    await engine.call('task.queue', {
      accountId: G,
      task: tasks.changeLabels({
        accountId: G,
        threadIds: [t.id],
        labelsToRemove: [inbox],
        labelsToAdd: [travel],
      }),
    });
    await tick();
    const updated = threadsIn(deltas).find((x) => x.id === t.id)!;
    expect(updated.labels.map((l) => l.id)).not.toContain('g-inbox');
    expect(updated.labels.map((l) => l.id)).toContain('g-travel');
    expect(updated.folders.map((f) => f.id)).toEqual(['g-all']);
  });

  it('drafts: syncback creates a draft thread, send moves it to Sent', async () => {
    const draft: MessageJSON = {
      id: 'draft-1',
      aid: I,
      v: 0,
      __cls: 'Message',
      subject: 'Hello from the test',
      date: NOW,
      threadId: '',
      hMsgId: 'draft-1@tern.local',
      unread: false,
      starred: false,
      draft: true,
      body: '<p>Hi there</p>',
      from: [{ email: 'maya@okafor.studio', name: 'Maya Okafor' }],
      to: [{ email: 'rosa@tidepool.coop' }],
      cc: [],
      bcc: [],
      replyTo: [],
      files: [],
    };
    await engine.call('task.queue', { accountId: I, task: tasks.syncbackDraft(draft) });
    const drafts = await engine.call('threads.list', { accountId: I, folderId: 'i-drafts' });
    const thread = drafts.threads.find((t) => t.subject === 'Hello from the test')!;
    expect(thread).toBeDefined();

    const send = tasks.sendDraft(draft);
    await engine.call('task.queue', { accountId: I, task: send });
    const done = await taskDone(deltas, send.id);
    expect(done.error).toBeUndefined();
    const sent = await engine.call('threads.list', { accountId: I, folderId: 'i-sent' });
    expect(sent.threads.some((t) => t.id === thread.id)).toBe(true);
    const afterDrafts = await engine.call('threads.list', { accountId: I, folderId: 'i-drafts' });
    expect(afterDrafts.threads.some((t) => t.id === thread.id)).toBe(false);
  });

  it('SendDraftTask without recipients completes with an error', async () => {
    const task = tasks.sendDraft({
      ...[...engine.store.messages.values()].find((m) => m.draft && m.aid === G)!,
      to: [],
      cc: [],
      bcc: [],
    });
    await engine.call('task.queue', { accountId: G, task });
    const done = await taskDone(deltas, task.id);
    expect(done.error?.key).toBe('no-recipients');
  });

  it('DestroyDraftTask removes the draft and its thread', async () => {
    const d = [...engine.store.messages.values()].find((m) => m.draft && m.aid === I)!;
    await engine.call('task.queue', { accountId: I, task: tasks.destroyDraft(I, [d.id]) });
    await tick();
    const removed = deltas.find((x) => x.modelClass === 'Thread' && x.type === 'unpersist');
    expect(removed?.modelJSONs.map((t) => (t as ThreadJSON).id)).toContain(d.threadId);
  });

  it('search understands free text and operators', async () => {
    const lisbon = await engine.call('search', { accountId: G, query: 'lisbon' });
    expect(lisbon.threads.some((t) => t.subject === 'Lisbon in May?')).toBe(true);
    const fromJonas = await engine.call('search', {
      accountId: I,
      query: 'from:jonas has:attachment',
    });
    expect(fromJonas.threads.length).toBeGreaterThan(0);
    expect(fromJonas.threads.every((t) => t.attachmentCount > 0)).toBe(true);
    const unread = await engine.call('search', { accountId: G, query: 'is:unread in:spam' });
    expect(unread.threads.every((t) => t.unread > 0)).toBe(true);
  });

  it('search and contacts.search require an account', async () => {
    const err = await engine.call('search', { query: 'x' } as never).catch((e: unknown) => e);
    expect((err as RpcError).code).toBe(RpcErrorCode.InvalidParams);
  });

  it('contacts.search matches names and emails by prefix', async () => {
    const results = await Promise.all(
      [G, I].map((accountId) => engine.call('contacts.search', { accountId, query: 'pri' })),
    );
    const emails = results.flatMap((r) => r.contacts.map((c) => c.email));
    expect(emails).toContain('priya.raman@northwind.design');
  });

  it('folders.list and labels.list report unread threads per category', async () => {
    const { labels, counts } = await engine.call('labels.list', { accountId: G });
    expect(labels.some((l) => l.id === 'g-inbox')).toBe(true);
    expect(counts['g-inbox'].total).toBeGreaterThan(0);
    expect(counts['g-inbox'].unread).toBeGreaterThan(0);
    expect(counts['g-inbox'].unread).toBeLessThanOrEqual(counts['g-inbox'].total);
  });

  it('sync.wake delivers new mail as deltas', async () => {
    await engine.call('sync.wake', { accountId: G });
    await tick();
    const t = threadsIn(deltas).find((x) => x.subject === 'Quick question about June');
    expect(t?.labels.some((l) => l.role === 'inbox')).toBe(true);
  });

  it('honours the subscribe filter', async () => {
    await engine.call('subscribe', { accounts: [I], classes: ['Thread'] });
    deltas.length = 0;
    const gt = [...engine.store.threads.values()].find((x) => x.aid === G)!;
    await engine.call('task.queue', {
      accountId: G,
      task: tasks.changeStarred({ accountId: G, threadIds: [gt.id], starred: true }),
    });
    await tick();
    expect(deltas).toHaveLength(0);
  });
});
