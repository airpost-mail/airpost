import { describe, expect, it } from 'vitest';
import type { CategoryJSON, MessageJSON, ThreadJSON } from '@/engine';
import { labelHue } from './labels';
import { resolveMailbox } from './mailbox';
import { foldRuns, initialExpansion } from './reader/expansion';
import { senderLabel } from './list/senders';
import { mergeAccountPages } from './use-unified-threads';

const thread = (id: string, aid: string, lmt: number, extra: Partial<ThreadJSON> = {}) =>
  ({ id, aid, lmt, participants: [], folders: [], labels: [], ...extra }) as unknown as ThreadJSON;
const pages = (threads: ThreadJSON[], next: string | null) => ({
  pages: [{ threads, next }],
  pageParams: [null],
});
const msg = (id: string, unread = false, draft = false) => ({ id, unread, draft }) as MessageJSON;

describe('unified merge', () => {
  it('sorts across accounts and hides what an account with more pages has not loaded', () => {
    const a = pages([thread('a1', 'A', 100), thread('a2', 'A', 50)], 'opaque');
    const b = pages([thread('b1', 'B', 90), thread('b2', 'B', 10)], null);
    const merged = mergeAccountPages([a, b]);
    expect(merged.threads.map((t) => t.id)).toEqual(['a1', 'b1', 'a2']);
    expect(merged.hasNextPage).toBe(true);
  });

  it('shows everything once no account has more', () => {
    const a = pages([thread('a1', 'A', 100)], null);
    const b = pages([thread('b1', 'B', 10)], null);
    expect(mergeAccountPages([a, b, undefined]).threads.map((t) => t.id)).toEqual(['a1', 'b1']);
  });
});

describe('mailbox resolution', () => {
  const inbox = {
    id: 'f1',
    aid: 'A',
    __cls: 'Folder',
    role: 'inbox',
    path: 'INBOX',
  } as CategoryJSON;
  it('resolves categories, starred and unified views', () => {
    expect(resolveMailbox('A', 'f1', [inbox])).toMatchObject({ kind: 'category' });
    expect(resolveMailbox('A', 'starred', [inbox])).toEqual({ kind: 'starred', accountId: 'A' });
    expect(resolveMailbox('all', 'inbox', undefined)).toEqual({ kind: 'unified', role: 'inbox' });
    expect(resolveMailbox('all', 'Work', undefined)).toBeNull();
    expect(resolveMailbox('A', 'nope', [inbox])).toBeNull();
    expect(resolveMailbox('A', 'f1', undefined)).toBeUndefined();
  });
});

describe('reader expansion', () => {
  it('opens unread messages, drafts and the last one', () => {
    const snap = initialExpansion([
      msg('1'),
      msg('2', true),
      msg('3'),
      msg('4', false, true),
      msg('5'),
    ]);
    expect([...snap.open].sort()).toEqual(['2', '4', '5']);
  });

  it('folds three or more collapsed messages after the first', () => {
    const ms = ['1', '2', '3', '4', '5', '6'].map((id) => msg(id));
    const open = new Set(['6']);
    const runs = foldRuns(ms, (m) => open.has(m.id));
    expect(
      runs.map((r) => (r.kind === 'folded' ? `fold:${r.messages.length}` : r.message.id)),
    ).toEqual(['1', 'fold:4', '6']);
    const short = foldRuns(ms.slice(0, 3), () => false);
    expect(short.every((r) => r.kind === 'message')).toBe(true);
  });
});

describe('row text', () => {
  it('names other participants, or me', () => {
    const own = new Set(['me@x.test']);
    const t = (emails: string[]) =>
      thread('t', 'A', 0, {
        participants: emails.map((email) => ({ email, name: email.split('@')[0] + ' Z' })),
      });
    expect(senderLabel(t(['me@x.test']), own)).toBe('me');
    expect(senderLabel(t(['me@x.test', 'jo@x.test']), own)).toBe('jo Z');
    expect(senderLabel(t(['a@x.test', 'b@x.test', 'c@x.test']), own)).toBe('a, b +1');
  });

  it('gives a label the same hue as the desktop', () => {
    expect(labelHue({ path: 'Travel', role: '' })).toBeCloseTo(
      ((84 + 114 + 97 + 118 + 101 + 108) * 396) / 512,
    );
  });
});
