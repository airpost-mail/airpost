import { QueryClient } from '@tanstack/react-query';
import { describe, expect, it } from 'vitest';
import { engineKeys, EVERY_ACCOUNT, type ThreadQuery } from './keys';
import type { LiveSnapshot } from './live';
import { applyOptimistic, optimisticThreads } from './optimistic';
import * as tasks from './tasks';
import type { FolderJSON, ThreadCategoryRef, ThreadJSON } from './types';

const A = 'acc1';
const inbox: ThreadCategoryRef = {
  id: 'inbox',
  aid: A,
  v: 1,
  __cls: 'Folder',
  path: 'INBOX',
  role: 'inbox',
  _refs: 1,
  _u: 1,
};
const archive: FolderJSON = {
  id: 'archive',
  aid: A,
  v: 1,
  __cls: 'Folder',
  path: 'Archive',
  role: 'archive',
};

function thread(id: string, lmt: number, over: Partial<ThreadJSON> = {}): ThreadJSON {
  return {
    id,
    aid: A,
    v: 1,
    __cls: 'Thread',
    subject: id,
    unread: 1,
    starred: 0,
    lmt,
    fmt: lmt,
    lmst: 0,
    lmrt: lmt,
    inAllMail: true,
    attachmentCount: 0,
    folders: [inbox],
    labels: [],
    participants: [],
    ...over,
  };
}

const list = (...rows: ThreadJSON[]): LiveSnapshot<ThreadJSON> => ({
  rows,
  total: rows.length,
  offset: 0,
});
const ids = (qc: QueryClient, q: ThreadQuery) =>
  qc.getQueryData<LiveSnapshot<ThreadJSON>>(engineKeys.threadList(q))!.rows.map((t) => t.id);

describe('applyOptimistic', () => {
  const q: ThreadQuery = { accountId: A, folderId: 'inbox' };

  it('patches rows in place and keeps their summary', () => {
    const qc = new QueryClient();
    const summary = {
      messageCount: 2,
      senders: [],
      hasDraft: false,
      sending: false,
      hasAttachment: false,
    };
    qc.setQueryData(
      engineKeys.threadList(q),
      list(thread('a', 300, { summary }), thread('b', 200)),
    );
    applyOptimistic(qc, A, [thread('a', 300, { unread: 0 })]);
    const rows = qc.getQueryData<LiveSnapshot<ThreadJSON>>(engineKeys.threadList(q))!.rows;
    expect(rows.map((t) => [t.id, t.unread])).toEqual([
      ['a', 0],
      ['b', 1],
    ]);
    expect(rows[0].summary).toEqual(summary);
  });

  it('takes a thread out of a list it leaves, and inserts nothing', () => {
    const qc = new QueryClient();
    const unified: ThreadQuery = {
      accountId: EVERY_ACCOUNT,
      categoryIds: ['inbox', 'other-inbox'],
      categoryRole: 'inbox',
    };
    const starred: ThreadQuery = { accountId: EVERY_ACCOUNT, starred: true };
    qc.setQueryData(engineKeys.threadList(unified), list(thread('a', 300), thread('b', 200)));
    qc.setQueryData(engineKeys.threadList(starred), list());
    applyOptimistic(qc, A, [thread('a', 300, { inAllMail: false, starred: 1 })]);
    expect(ids(qc, unified)).toEqual(['b']);
    expect(qc.getQueryData<LiveSnapshot<ThreadJSON>>(engineKeys.threadList(unified))!.total).toBe(
      1,
    );
    // The engine's live query adds it to Starred.
    expect(ids(qc, starred)).toEqual([]);
  });

  it("leaves another account's thread with the same id alone", () => {
    const qc = new QueryClient();
    const all: ThreadQuery = { accountId: EVERY_ACCOUNT, starred: false };
    qc.setQueryData(engineKeys.threadList(all), list({ ...thread('a', 300), aid: 'other' }));
    applyOptimistic(qc, A, [thread('a', 300, { starred: 1 })]);
    expect(ids(qc, all)).toEqual(['a']);
  });
});

describe('optimisticThreads', () => {
  const ctx = (t: ThreadJSON, gmail = false) => ({
    thread: (id: string) => (id === t.id ? t : undefined),
    isGmail: () => gmail,
  });

  it('marks read and clears per-folder unread', () => {
    const t = thread('a', 1);
    const [next] = optimisticThreads(
      tasks.changeUnread({ accountId: A, threadIds: ['a'], unread: false }),
      ctx(t),
    );
    expect(next.unread).toBe(0);
    expect(next.folders[0]._u).toBe(0);
  });

  it('moves out of the inbox on IMAP', () => {
    const t = thread('a', 1);
    const [next] = optimisticThreads(
      tasks.changeFolder({ accountId: A, threadIds: ['a'], folder: archive }),
      ctx(t),
    );
    expect(next.folders.map((f) => f.id)).toEqual(['archive']);
  });

  it('ignores tasks without threads', () => {
    expect(optimisticThreads(tasks.destroyDraft(A, ['m']), ctx(thread('a', 1)))).toEqual([]);
  });
});
