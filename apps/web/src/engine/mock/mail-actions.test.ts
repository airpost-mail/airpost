import { beforeEach, describe, expect, it } from 'vitest';
import { nowSeconds, threadHasRole } from '../model';
import { RpcError, RpcErrorCode } from '../rpc';
import type { MessageJSON, SendDraftTaskJSON, ThreadJSON } from '../types';
import { MockEngine } from './engine';
import { MOCK_ACCOUNT_IDS } from './fixtures';
import { chooseUnsubscribe, parseMailto } from './mail-actions';

const G = MOCK_ACCOUNT_IDS.gmail;
const I = MOCK_ACCOUNT_IDS.imap;
const tick = (ms = 5) => new Promise((r) => setTimeout(r, ms));

const code = (p: Promise<unknown>) =>
  p.then(
    () => null,
    (e: unknown) => (e instanceof RpcError ? e.code : e),
  );

const snoozeOf = (t: ThreadJSON) =>
  (t.metadata?.find((m) => m.pluginId === 'thread-snooze')?.value as { expiration: unknown })
    ?.expiration;

describe('mock mail actions', () => {
  let engine: MockEngine;
  beforeEach(async () => {
    engine = new MockEngine({ latency: 0 });
    await engine.connect();
  });

  const inboxThread = async (accountId: string) => {
    const page = await engine.call('threads.list', { accountId, role: 'inbox' });
    return page.threads[0]!;
  };
  const stored = (id: string) => engine.store.threads.get(id)!;

  it('snoozes an IMAP thread into the Snoozed folder and back', async () => {
    const t = await inboxThread(I);
    const until = nowSeconds() + 3600;
    const r = await engine.call('threads.snooze', { accountId: I, threadIds: [t.id], until });
    expect(r.taskIds).toHaveLength(2);
    const snoozed = stored(t.id);
    expect(threadHasRole(snoozed, 'snoozed')).toBe(true);
    expect(threadHasRole(snoozed, 'inbox')).toBe(false);
    expect(snoozeOf(snoozed)).toBe(until);

    const back = await engine.call('threads.unsnooze', { accountId: I, threadIds: [t.id] });
    expect(back.taskIds).toHaveLength(2);
    expect(threadHasRole(stored(t.id), 'inbox')).toBe(true);
    expect(threadHasRole(stored(t.id), 'snoozed')).toBe(false);
    expect(snoozeOf(stored(t.id))).toBeNull();
    engine.close();
  });

  it('snoozes on Gmail by swapping the Inbox label for Snoozed', async () => {
    const t = await inboxThread(G);
    await engine.call('threads.snooze', {
      accountId: G,
      threadIds: [t.id],
      until: nowSeconds() + 3600,
    });
    const labels = stored(t.id).labels.map((l) => l.id);
    expect(labels).toContain('g-snoozed');
    expect(labels).not.toContain('g-inbox');
    engine.close();
  });

  it('brings the thread back when the snooze comes due', async () => {
    const t = await inboxThread(I);
    await engine.call('threads.snooze', {
      accountId: I,
      threadIds: [t.id],
      until: nowSeconds() + 1,
    });
    expect(threadHasRole(stored(t.id), 'snoozed')).toBe(true);
    const start = Date.now();
    while (!threadHasRole(stored(t.id), 'inbox') && Date.now() - start < 2500) await tick(50);
    expect(threadHasRole(stored(t.id), 'inbox')).toBe(true);
    expect(snoozeOf(stored(t.id))).toBeNull();
    engine.close();
  });

  it('refuses a past time, unknown threads, and engines without the feature', async () => {
    const t = await inboxThread(I);
    expect(
      await code(engine.call('threads.snooze', { accountId: I, threadIds: [t.id], until: 1000 })),
    ).toBe(RpcErrorCode.InvalidParams);
    expect(
      await code(
        engine.call('threads.snooze', {
          accountId: I,
          threadIds: ['nope'],
          until: nowSeconds() + 60,
        }),
      ),
    ).toBe(RpcErrorCode.NotFound);
    expect(await code(engine.call('threads.unsnooze', { accountId: I, threadIds: [] }))).toBe(
      RpcErrorCode.NotFound,
    );
    const old = new MockEngine({ latency: 0, features: ['query'] });
    await old.connect();
    expect(await code(old.call('threads.unsnooze', { accountId: I, threadIds: [t.id] }))).toBe(
      RpcErrorCode.MethodNotFound,
    );
    expect(await code(old.call('messages.unsubscribe', { accountId: I, messageId: 'x' }))).toBe(
      RpcErrorCode.MethodNotFound,
    );
  });

  const listMessage = () =>
    [...engine.store.messages.values()].find((m) => m.hListUnsubPost && m.aid === I)!;

  it('unsubscribes by one-click, or hands back the web link', async () => {
    const m = listMessage();
    const r = await engine.call('messages.unsubscribe', { accountId: I, messageId: m.id });
    expect(r).toEqual({ method: 'one-click', ok: true, url: 'https://unsubscribe.example/u/123' });
    const web = await engine.call('messages.unsubscribe', {
      accountId: I,
      messageId: m.id,
      method: 'web',
    });
    expect(web.method).toBe('web');
    expect(
      await code(
        engine.call('messages.unsubscribe', { accountId: I, messageId: m.id, method: 'mailto' }),
      ),
    ).toBe(RpcErrorCode.NotFound);
    expect(
      await code(
        engine.call('messages.unsubscribe', {
          accountId: I,
          messageId: m.id,
          method: 'body-link' as never,
        }),
      ),
    ).toBe(RpcErrorCode.InvalidParams);
  });

  it('sends the mailto message from the address the list wrote to', async () => {
    const m = listMessage();
    const stored: MessageJSON = engine.store.messages.get(m.id)!;
    stored.hListUnsub = '<mailto:leave@lists.example.net?subject=stop>';
    delete stored.hListUnsubPost;
    const r = await engine.call('messages.unsubscribe', { accountId: I, messageId: m.id });
    expect(r).toMatchObject({ method: 'mailto', ok: true });
    const task = engine.store.tasks.get(r.taskId!) as SendDraftTaskJSON;
    expect(task.draft.to).toEqual([{ name: '', email: 'leave@lists.example.net' }]);
    expect(task.draft.subject).toBe('stop');
    expect(task.draft.body).toBe('unsubscribe');
    expect(task.draft.plaintext).toBe(true);
    await tick();
    const sent = [...engine.store.messages.values()].find(
      (x) => x.hMsgId === task.headerMessageId && !x.draft,
    );
    expect(sent?.to[0]?.email).toBe('leave@lists.example.net');

    stored.hListUnsub = '<mailto:not-an-address>';
    const bad = await engine.call('messages.unsubscribe', { accountId: I, messageId: m.id });
    expect(bad).toMatchObject({ method: 'mailto', ok: false });
    expect(bad.error).toContain('not an email address');
  });
});

describe('chooseUnsubscribe', () => {
  const list = '<mailto:leave@x.example?subject=u>, \n <https://x.example/u/1>';
  const post = 'List-Unsubscribe=One-Click';

  it('prefers one-click over https, then mailto, then web, as the engine does', () => {
    expect(chooseUnsubscribe(list, post)).toEqual({
      method: 'one-click',
      url: 'https://x.example/u/1',
    });
    expect(chooseUnsubscribe(list, undefined)?.method).toBe('mailto');
    expect(chooseUnsubscribe('<http://x.example/u>', post)).toEqual({
      method: 'web',
      url: 'http://x.example/u',
    });
    expect(chooseUnsubscribe('<http://x.example/u>', post, 'one-click')).toBeNull();
    expect(chooseUnsubscribe(undefined, post)).toBeNull();
  });

  it('reads mailto fields and refuses what the engine refuses', () => {
    expect(parseMailto('MAILTO:a%2Bb@x.example?Subject=Please%20go&cc=c@y.example')).toEqual({
      to: ['a+b@x.example'],
      cc: ['c@y.example'],
      subject: 'Please go',
      body: '',
    });
    for (const bad of ['mailto:', 'mailto:nobody', 'mailto:a%ZZ@x.io', 'https://x.example']) {
      expect(() => parseMailto(bad), bad).toThrow();
    }
  });
});
