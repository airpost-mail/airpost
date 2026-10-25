import { QueryClient } from '@tanstack/react-query';
import { beforeEach, describe, expect, it } from 'vitest';
import { applyDelta, upsertThreadInPages, type ThreadPages } from './cache';
import { engineKeys, EVERY_ACCOUNT, type ThreadQuery } from './keys';
import { optimisticThreads } from './optimistic';
import type { FoldersListResult } from './rpc';
import * as tasks from './tasks';
import type { FolderJSON, MessageJSON, ThreadCategoryRef, ThreadJSON } from './types';

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

function message(id: string, threadId: string, date: number, body?: string): MessageJSON {
  return {
    id,
    aid: A,
    v: 1,
    __cls: 'Message',
    subject: 's',
    date,
    threadId,
    hMsgId: `${id}@x`,
    unread: false,
    starred: false,
    draft: false,
    body,
    from: [],
    to: [],
    cc: [],
    bcc: [],
    replyTo: [],
    files: [],
  };
}

const q: ThreadQuery = { accountId: A, folderId: 'inbox' };

function pages(...chunks: [ThreadJSON[], boolean][]): ThreadPages {
  return {
    pages: chunks.map(([threads, more]) => ({
      threads,
      next: more ? `${threads[threads.length - 1].lmt}:${threads[threads.length - 1].id}` : null,
    })),
    pageParams: chunks.map(() => null),
  };
}

describe('upsertThreadInPages', () => {
  it('inserts a new matching thread in sort order', () => {
    const data = pages([[thread('a', 300), thread('c', 100)], false]);
    const next = upsertThreadInPages(data, thread('b', 200), q)!;
    expect(next.pages[0].threads.map((t) => t.id)).toEqual(['a', 'b', 'c']);
  });

  it('removes a thread that no longer matches', () => {
    const data = pages([[thread('a', 300), thread('b', 200)], false]);
    const moved = thread('a', 300, { folders: [{ ...archive, _refs: 1, _u: 0 }] });
    expect(upsertThreadInPages(data, moved, q)!.pages[0].threads.map((t) => t.id)).toEqual(['b']);
  });

  it('moves an updated thread to its new position', () => {
    const data = pages([[thread('a', 300), thread('b', 200)], false]);
    const bumped = thread('b', 400);
    expect(upsertThreadInPages(data, bumped, q)!.pages[0].threads.map((t) => t.id)).toEqual([
      'b',
      'a',
    ]);
  });

  it('leaves out threads past the loaded window when more pages exist', () => {
    const data = pages([[thread('a', 300), thread('b', 200)], true]);
    const old = thread('z', 10);
    expect(upsertThreadInPages(data, old, q)).toBe(data);
  });

  it('places threads into the right page', () => {
    const data = pages([[thread('a', 500), thread('b', 400)], true], [[thread('c', 200)], false]);
    const next = upsertThreadInPages(data, thread('x', 300), q)!;
    expect(next.pages[0].threads.map((t) => t.id)).toEqual(['a', 'b']);
    expect(next.pages[1].threads.map((t) => t.id)).toEqual(['x', 'c']);
  });
});

describe('query-backed lists', () => {
  it('drop a thread that left all mail from a list without a role', () => {
    const custom: ThreadCategoryRef = {
      ...inbox,
      id: 'l-work',
      __cls: 'Label',
      path: 'Work',
      role: '',
    };
    const label: ThreadQuery = { accountId: A, labelId: 'l-work' };
    const labelled = thread('a', 300, { labels: [{ ...custom, _refs: 1, _u: 0 }] });
    const data = pages([
      [labelled, thread('b', 200, { labels: [{ ...custom, _refs: 1, _u: 0 }] })],
      false,
    ]);
    const trashed = { ...labelled, inAllMail: false };
    expect(upsertThreadInPages(data, trashed, label)!.pages[0].threads.map((t) => t.id)).toEqual([
      'b',
    ]);
    // Trash itself still lists it.
    const trash: ThreadQuery = { accountId: A, folderId: 'trash', categoryRole: 'trash' };
    const inTrash = thread('a', 300, {
      inAllMail: false,
      folders: [{ ...inbox, id: 'trash', path: 'Trash', role: 'trash', _refs: 1, _u: 0 }],
    });
    expect(
      upsertThreadInPages(pages([[], false]), inTrash, trash)!.pages[0].threads.map((t) => t.id),
    ).toEqual(['a']);
  });

  it('insert by the sort the page was read with', () => {
    // b was received last but a's last message is newer (a sent reply).
    const a = thread('a', 500, { lmrt: 100, lmst: 500 });
    const b = thread('b', 200, { lmrt: 200, lmst: 0 });
    const byReceived = { ...pages([[a], false]) };
    byReceived.pages[0].sort = 'lmrt';
    expect(upsertThreadInPages(byReceived, b, q)!.pages[0].threads.map((t) => t.id)).toEqual([
      'b',
      'a',
    ]);
    expect(
      upsertThreadInPages(pages([[a], false]), b, q)!.pages[0].threads.map((t) => t.id),
    ).toEqual(['a', 'b']);
  });

  it('keep the lists across accounts in step, by category and flags', () => {
    const qc = new QueryClient();
    const unified: ThreadQuery = {
      accountId: EVERY_ACCOUNT,
      categoryIds: ['inbox', 'other-inbox'],
      categoryRole: 'inbox',
    };
    const starred: ThreadQuery = { accountId: EVERY_ACCOUNT, starred: true };
    qc.setQueryData(engineKeys.threadList(unified), pages([[thread('a', 300)], false]));
    qc.setQueryData(engineKeys.threadList(starred), pages([[], false]));
    const other = {
      ...thread('o', 400, { starred: 1 }),
      aid: 'other',
      folders: [{ ...inbox, id: 'other-inbox', aid: 'other' }],
    };
    applyDelta(qc, {
      accountId: 'other',
      modelClass: 'Thread',
      type: 'persist',
      modelJSONs: [other],
    });
    const ids = (k: ThreadQuery) =>
      qc.getQueryData<ThreadPages>(engineKeys.threadList(k))!.pages[0].threads.map((t) => t.id);
    expect(ids(unified)).toEqual(['o', 'a']);
    expect(ids(starred)).toEqual(['o']);
    // Out of all mail (trashed) leaves the inbox list.
    applyDelta(qc, {
      accountId: A,
      modelClass: 'Thread',
      type: 'persist',
      modelJSONs: [thread('a', 300, { inAllMail: false })],
    });
    expect(ids(unified)).toEqual(['o']);
  });
});

describe('applyDelta', () => {
  let qc: QueryClient;
  beforeEach(() => {
    qc = new QueryClient();
  });

  it('patches thread lists and the single-thread entry', () => {
    qc.setQueryData(engineKeys.threadList(q), pages([[thread('a', 300)], false]));
    qc.setQueryData(engineKeys.thread(A, 'a'), thread('a', 300));
    applyDelta(qc, {
      accountId: A,
      modelClass: 'Thread',
      type: 'persist',
      modelJSONs: [thread('a', 300, { unread: 0 }), thread('n', 400)],
    });
    const list = qc.getQueryData<ThreadPages>(engineKeys.threadList(q))!;
    expect(list.pages[0].threads.map((t) => [t.id, t.unread])).toEqual([
      ['n', 1],
      ['a', 0],
    ]);
    expect(qc.getQueryData<ThreadJSON>(engineKeys.thread(A, 'a'))?.unread).toBe(0);
  });

  it('removes unpersisted threads everywhere', () => {
    qc.setQueryData(engineKeys.threadList(q), pages([[thread('a', 300), thread('b', 200)], false]));
    qc.setQueryData(engineKeys.thread(A, 'a'), thread('a', 300));
    applyDelta(qc, {
      accountId: A,
      modelClass: 'Thread',
      type: 'unpersist',
      modelJSONs: [thread('a', 300)],
    });
    expect(qc.getQueryData<ThreadPages>(engineKeys.threadList(q))!.pages[0].threads).toHaveLength(
      1,
    );
    expect(qc.getQueryData(engineKeys.thread(A, 'a'))).toBeNull();
  });

  it('ignores deltas for other accounts', () => {
    const data = pages([[thread('a', 300)], false]);
    qc.setQueryData(engineKeys.threadList(q), data);
    applyDelta(qc, {
      accountId: 'other',
      modelClass: 'Thread',
      type: 'persist',
      modelJSONs: [{ ...thread('x', 900), aid: 'other' }],
    });
    expect(qc.getQueryData(engineKeys.threadList(q))).toEqual(data);
  });

  it('merges messages and keeps a downloaded body', () => {
    qc.setQueryData(engineKeys.messages(A, 't1'), [message('m1', 't1', 1, '<p>hi</p>')]);
    applyDelta(qc, {
      accountId: A,
      modelClass: 'Message',
      type: 'persist',
      modelJSONs: [{ ...message('m1', 't1', 1), unread: true }, message('m2', 't1', 2)],
    });
    const list = qc.getQueryData<MessageJSON[]>(engineKeys.messages(A, 't1'))!;
    expect(list.map((m) => m.id)).toEqual(['m1', 'm2']);
    expect(list[0].body).toBe('<p>hi</p>');
    expect(list[0].unread).toBe(true);
  });

  it('drops the cached body of a draft that changed, so it is read again', () => {
    const draft = { ...message('d1', 't1', 1, '<p>old</p>'), draft: true };
    qc.setQueryData(engineKeys.messages(A, 't1'), [draft]);
    const saved = { ...message('d1', 't1', 1), draft: true, v: 2 };
    applyDelta(qc, { accountId: A, modelClass: 'Message', type: 'persist', modelJSONs: [saved] });
    expect(qc.getQueryData<MessageJSON[]>(engineKeys.messages(A, 't1'))![0].body).toBeUndefined();
  });

  it('treats metadata-expiration as no change', () => {
    const data = pages([[thread('a', 300)], false]);
    qc.setQueryData(engineKeys.threadList(q), data);
    qc.setQueryData(engineKeys.thread(A, 'a'), thread('a', 300));
    applyDelta(qc, {
      accountId: A,
      modelClass: 'Thread',
      type: 'metadata-expiration',
      modelJSONs: [thread('a', 300)],
    });
    expect(qc.getQueryData(engineKeys.threadList(q))).toBe(data);
    expect(qc.getQueryData<ThreadJSON>(engineKeys.thread(A, 'a'))?.id).toBe('a');
  });

  it('drops a message that moved to another thread', () => {
    qc.setQueryData(engineKeys.messages(A, 't1'), [message('m1', 't1', 1)]);
    applyDelta(qc, {
      accountId: A,
      modelClass: 'Message',
      type: 'persist',
      modelJSONs: [message('m1', 't2', 1)],
    });
    expect(qc.getQueryData<MessageJSON[]>(engineKeys.messages(A, 't1'))).toEqual([]);
  });

  it('keeps counts by ThreadCounts deltas, not by refetching the category lists', () => {
    qc.setQueryData(engineKeys.counts(), { inbox: { unread: 2, total: 5 } });
    qc.setQueryData<FoldersListResult>(engineKeys.folders(A), { folders: [archive], counts: {} });
    applyDelta(qc, {
      accountId: A,
      modelClass: 'Thread',
      type: 'persist',
      modelJSONs: [thread('a', 300)],
    });
    expect(qc.getQueryState(engineKeys.folders(A))?.isInvalidated).toBe(false);
    applyDelta(qc, {
      accountId: A,
      modelClass: 'ThreadCounts',
      type: 'persist',
      modelJSONs: [
        { id: 'inbox', categoryId: 'inbox', unread: 1, total: 5 },
        { id: 'archive', categoryId: 'archive', unread: 0, total: 1 },
      ],
    });
    expect(qc.getQueryData(engineKeys.counts())).toEqual({
      inbox: { unread: 1, total: 5 },
      archive: { unread: 0, total: 1 },
    });
    applyDelta(qc, {
      accountId: A,
      modelClass: 'ThreadCounts',
      type: 'unpersist',
      modelJSONs: [{ id: 'archive', categoryId: 'archive' }],
    });
    expect(qc.getQueryData(engineKeys.counts())).toEqual({ inbox: { unread: 1, total: 5 } });
  });

  it('upserts folders and tracks tasks', () => {
    qc.setQueryData<FoldersListResult>(engineKeys.folders(A), { folders: [archive], counts: {} });
    const created: FolderJSON = { ...archive, id: 'f2', path: 'Clients', role: '' };
    applyDelta(qc, { accountId: A, modelClass: 'Folder', type: 'persist', modelJSONs: [created] });
    const result = qc.getQueryData<FoldersListResult>(engineKeys.folders(A))!;
    expect(result.folders.map((f) => f.id)).toEqual(['archive', 'f2']);

    const t = {
      ...tasks.changeUnread({ accountId: A, threadIds: ['a'], unread: false }),
      status: 'complete' as const,
    };
    applyDelta(qc, { accountId: A, modelClass: 'Task', type: 'persist', modelJSONs: [t] });
    expect(qc.getQueryData(engineKeys.tasks(A))).toEqual([t]);
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
