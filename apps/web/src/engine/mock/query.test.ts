import { beforeEach, describe, expect, it } from 'vitest';
import { fetchThreadPage, threadListQuery } from '../hooks';
import { EVERY_ACCOUNT, type ThreadQuery } from '../keys';
import { compareThreads } from '../model';
import { RpcErrorCode, type QueryParams } from '../rpc';
import type { ContactJSON, EventJSON, ThreadJSON } from '../types';
import { MockEngine } from './engine';
import { MOCK_ACCOUNT_IDS } from './fixtures';

const NOW = 1_780_000_000;
const G = MOCK_ACCOUNT_IDS.gmail;
const I = MOCK_ACCOUNT_IDS.imap;

const ids = (rows: { id: string }[]) => rows.map((r) => r.id);

async function pages(engine: MockEngine, q: ThreadQuery, limit: number): Promise<ThreadJSON[]> {
  const out: ThreadJSON[] = [];
  let next: string | null = null;
  for (let i = 0; i < 100; i++) {
    const page = await fetchThreadPage(engine, q, limit, next);
    out.push(...page.threads);
    next = page.next;
    if (!next) return out;
  }
  throw new Error('no last page');
}

describe('MockEngine query', () => {
  let engine: MockEngine;

  beforeEach(async () => {
    engine = new MockEngine({ latency: 0, now: NOW });
    await engine.connect();
  });

  it('announces query, and an engine without it refuses the method', async () => {
    expect(engine.features?.has('query')).toBe(true);
    const old = new MockEngine({ latency: 0, now: NOW, features: [] });
    await old.connect();
    expect(old.features?.size).toBe(0);
    await expect(old.call('query', { class: 'Thread' })).rejects.toMatchObject({
      code: RpcErrorCode.MethodNotFound,
    });
  });

  it('lists a category as threads.list does, newest received first, by cursor', async () => {
    const inbox: ThreadQuery = { accountId: I, folderId: 'i-inbox', categoryRole: 'inbox' };
    const viaQuery = await pages(engine, inbox, 4);
    const listed = await engine.call('threads.list', {
      accountId: I,
      folderId: 'i-inbox',
      limit: 500,
    });
    expect(ids(viaQuery).sort()).toEqual(ids(listed.threads).sort());
    expect(viaQuery.length).toBeGreaterThan(4);
    expect(viaQuery).toEqual([...viaQuery].sort((a, b) => compareThreads(a, b, 'lmrt')));
  });

  it('sorts Sent by last sent', async () => {
    const sent = await pages(engine, { accountId: I, folderId: 'i-sent', categoryRole: 'sent' }, 3);
    expect(sent.length).toBeGreaterThan(1);
    expect(sent).toEqual([...sent].sort((a, b) => compareThreads(a, b, 'lmst')));
  });

  it('pages starred and unread across accounts', async () => {
    const all = [...engine.store.threads.values()].filter((t) => t.inAllMail);
    const starred = await pages(engine, { accountId: EVERY_ACCOUNT, starred: true }, 2);
    expect(ids(starred).sort()).toEqual(ids(all.filter((t) => t.starred > 0)).sort());
    expect(new Set(starred.map((t) => t.aid))).toEqual(new Set([G, I]));
    const unread = await pages(engine, { accountId: G, unread: true }, 5);
    expect(ids(unread).sort()).toEqual(ids(all.filter((t) => t.aid === G && t.unread > 0)).sort());
  });

  it('reads a unified list as one, and an empty one as nothing', async () => {
    const q: ThreadQuery = {
      accountId: EVERY_ACCOUNT,
      categoryIds: ['g-inbox', 'i-inbox'],
      categoryRole: 'inbox',
    };
    const unified = await pages(engine, q, 7);
    const one = async (accountId: string, id: string) =>
      ids(await pages(engine, { accountId, folderId: id, labelId: undefined }, 50));
    const g = (await engine.call('threads.list', { accountId: G, labelId: 'g-inbox', limit: 500 }))
      .threads;
    expect(ids(unified).sort()).toEqual([...ids(g), ...(await one(I, 'i-inbox'))].sort());
    expect(await pages(engine, { ...q, categoryIds: [] }, 7)).toEqual([]);
  });

  it('falls back to threads.list without query, filtering starred client-side', async () => {
    const old = new MockEngine({ latency: 0, now: NOW, features: [] });
    await old.connect();
    const q: ThreadQuery = { accountId: G, starred: true };
    const before = ids(await pages(old, q, 3)).sort();
    expect(before.length).toBeGreaterThan(0);
    expect(ids(await pages(engine, q, 3)).sort()).toEqual(before);
    await expect(fetchThreadPage(old, { accountId: EVERY_ACCOUNT }, 5, null)).rejects.toThrow(
      /needs an engine with `query`/,
    );
  });

  it('builds the desktop perspective query', () => {
    expect(
      threadListQuery({ accountId: G, labelId: 'g-trash', categoryRole: 'trash' }, 50, null),
    ).toEqual({
      class: 'Thread',
      accounts: [G],
      where: [{ attr: 'categories', op: 'containsAny', value: ['g-trash'] }],
      order: [{ attr: 'lastMessageReceivedTimestamp', dir: 'desc' }],
      limit: 50,
      cursor: '',
    });
    expect(threadListQuery({ accountId: G, role: 'inbox' }, 50, null)).toBeNull();
  });

  it('searches contacts across accounts, most referenced first, without hidden ones', async () => {
    const term = [...engine.store.contacts.values()][0].name.split(' ')[0].slice(0, 2);
    const r = await engine.call('query', {
      class: 'Contact',
      search: { text: term, grammar: false },
      where: [{ attr: 'hidden', op: '=', value: false }],
      order: [{ attr: 'refs', dir: 'desc' }],
    });
    const rows = r.rows as ContactJSON[];
    const want = [...engine.store.contacts.values()].filter(
      (c) =>
        !c.h &&
        `${c.name} ${c.email}`
          .toLowerCase()
          .split(/[^\p{L}\p{N}]+/u)
          .some((w) => w.startsWith(term.toLowerCase())),
    );
    expect(ids(rows).sort()).toEqual(ids(want).sort());
    expect(rows.map((c) => c.refs)).toEqual([...rows.map((c) => c.refs)].sort((a, b) => b - a));
  });

  it('reads calendars and the events of a range', async () => {
    const cals = await engine.call('query', { class: 'Calendar' });
    expect(cals.rows.length).toBe(3);
    const r = await engine.call('query', {
      class: 'Event',
      where: [
        { attr: 'calendarId', op: 'in', value: ['cal-i-work'] },
        { attr: 'recurrenceEnd', op: '>', value: NOW - 86400 },
        { attr: 'recurrenceStart', op: '<', value: NOW + 7 * 86400 },
      ],
      order: [{ attr: 'recurrenceStart', dir: 'asc' }],
    });
    const events = r.rows as EventJSON[];
    expect(events.length).toBeGreaterThan(1);
    expect(events.every((e) => e.cid === 'cal-i-work' && e.aid === I)).toBe(true);
    expect(events.map((e) => e.rs)).toEqual([...events.map((e) => e.rs)].sort((a, b) => a - b));
  });

  it('answers ids and counts, and a message with its body', async () => {
    const count = await engine.call('query', { class: 'Thread', accounts: [G], count: true });
    expect(count).toEqual({
      rows: [],
      count: [...engine.store.threads.values()].filter((t) => t.aid === G).length,
    });
    const m = [...engine.store.messages.values()].find((x) => engine.store.body(x.id) !== null)!;
    const r = await engine.call('query', {
      class: 'Message',
      where: { attr: 'id', op: '=', value: m.id },
      include: ['body'],
    });
    expect((r.rows[0] as { body: string }).body).toBe(engine.store.body(m.id));
    const idsOnly = await engine.call('query', {
      class: 'Message',
      where: { attr: 'threadId', op: '=', value: m.threadId },
      idsOnly: true,
    });
    expect(idsOnly.ids).toContain(m.id);
  });

  it('searches threads in the grammar, dates in the request zone', async () => {
    const r = await engine.call('query', {
      class: 'Thread',
      accounts: [G],
      search: { text: 'is:unread OR is:starred', grammar: true },
    });
    const rows = r.rows as ThreadJSON[];
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.every((t) => t.aid === G && (t.unread > 0 || t.starred > 0))).toBe(true);
    const day = async (tz: string) =>
      (
        await engine.call('query', {
          class: 'Thread',
          search: { text: 'after:today', grammar: true, tz, now: NOW },
          count: true,
        })
      ).count;
    // NOW is 2026-05-28 20:26 UTC: already the 29th in Auckland, so less of the mail is
    // from today there.
    expect(await day('Pacific/Auckland')).toBeLessThanOrEqual((await day('America/Los_Angeles'))!);
  });

  it('refuses what the engine refuses', async () => {
    const bad = async (p: unknown) =>
      engine.call('query', p as QueryParams).then(
        () => null,
        (e: { code: number; data?: unknown }) => e,
      );
    const old = new MockEngine({ latency: 0, now: NOW, features: ['query'] });
    await old.connect();
    expect(
      await old
        .call('query', { class: 'Thread', search: { text: 'from:ann', grammar: true } })
        .then(
          () => null,
          (e: { code: number; data?: unknown }) => e,
        ),
    ).toMatchObject({ code: RpcErrorCode.InvalidParams, data: { unsupported: 'search-grammar' } });
    for (const p of [
      { class: 'Contact', search: { text: 'ann', grammar: true } },
      { class: 'Thread', search: { text: '(ann', grammar: true } },
      { class: 'Thread', search: { text: '  ', grammar: false } },
      { class: 'Account' },
      { class: 'Thread', wher: [] },
      { class: 'Thread', where: { attr: 'snippet', op: '=', value: 'x' } },
      { class: 'Thread', where: { attr: 'categories', op: '=', value: 'x' } },
      { class: 'Thread', order: [{ attr: 'categories', dir: 'asc' }] },
    ]) {
      expect(await bad(p)).toMatchObject({ code: RpcErrorCode.InvalidParams });
    }
    expect(await bad({ class: 'Thread', accounts: ['nobody'] })).toMatchObject({
      code: RpcErrorCode.UnknownAccount,
    });
  });
});
