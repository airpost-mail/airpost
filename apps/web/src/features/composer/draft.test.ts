import { describe, expect, it } from 'vitest';
import type { MessageJSON } from '@/engine';
import {
  draftFromMessage,
  draftToMessage,
  fullBody,
  participantsForReply,
  replyDraft,
  subjectWithPrefix,
} from './draft';
import { sanitizeEditorHtml, sanitizeQuotedHtml } from './html';

const me = { name: 'Maya', email: 'maya@okafor.studio' };
const own = new Set(['maya@okafor.studio', 'hello@okafor.studio']);

function message(patch: Partial<MessageJSON> = {}): MessageJSON {
  return {
    id: 'm1',
    aid: 'acc',
    v: 1,
    __cls: 'Message',
    subject: 'Plans',
    date: 1_780_000_000,
    threadId: 't1',
    hMsgId: 'orig@x.io',
    unread: false,
    starred: false,
    draft: false,
    body: '<p>Hello <img src=x onerror="alert(1)"></p>',
    from: [{ name: 'Ann', email: 'ann@x.io' }],
    to: [me, { email: 'bob@x.io' }],
    cc: [{ email: 'cat@x.io' }, { email: 'HELLO@okafor.studio' }],
    bcc: [],
    replyTo: [],
    files: [],
    ...patch,
  };
}

describe('reply participants', () => {
  it('reply goes to the sender, reply-all adds everyone else but me', () => {
    expect(participantsForReply(message(), own, 'reply')).toEqual({
      to: [{ name: 'Ann', email: 'ann@x.io' }],
      cc: [],
    });
    expect(participantsForReply(message(), own, 'reply-all').cc).toEqual([
      { email: 'bob@x.io' },
      { email: 'cat@x.io' },
    ]);
  });

  it('prefers Reply-To unless it points at me', () => {
    const m = message({ replyTo: [{ email: 'list@x.io' }] });
    expect(participantsForReply(m, own, 'reply').to).toEqual([{ email: 'list@x.io' }]);
    const mine = message({ replyTo: [me] });
    expect(participantsForReply(mine, own, 'reply').to).toEqual([
      { name: 'Ann', email: 'ann@x.io' },
    ]);
  });

  it('replying to my own message goes to its recipients', () => {
    const m = message({ from: [me], to: [{ email: 'bob@x.io' }], cc: [{ email: 'cat@x.io' }] });
    expect(participantsForReply(m, own, 'reply-all')).toEqual({
      to: [{ email: 'bob@x.io' }],
      cc: [{ email: 'cat@x.io' }],
    });
  });

  it('prefixes subjects once', () => {
    expect(subjectWithPrefix('Plans', 'Re:')).toBe('Re: Plans');
    expect(subjectWithPrefix('RE: Plans', 'Re:')).toBe('Re: Plans');
    expect(subjectWithPrefix('Fwd: Plans', 'Re:')).toBe('Re: Plans');
  });
});

describe('draft bodies', () => {
  const identity = { accountId: 'acc', contact: me };

  it('quotes the original without active content and threads the reply', () => {
    const d = replyDraft({ message: message(), identity, own, mode: 'reply' });
    expect(d.quoted).toContain('gmail_quote');
    expect(d.quoted).not.toContain('onerror');
    const m = draftToMessage({ ...d, body: '<div>Sounds good</div>' });
    expect(m).toMatchObject({ rthMsgId: 'orig@x.io', threadId: 't1', draft: true, aid: 'acc' });
    expect(m.body).toMatch(/^<div>Sounds good<\/div><br><br><div class="gmail_quote_attribution">/);
  });

  it('waits for the body before quoting', () => {
    const d = replyDraft({ message: message({ body: null }), identity, own, mode: 'reply' });
    expect(d.awaitingQuote).toBe(true);
    expect(fullBody(d)).toBe('');
  });

  it('reopens a saved reply with the quote split off', () => {
    const d = replyDraft({ message: message(), identity, own, mode: 'reply' });
    const saved = draftToMessage({ ...d, body: '<div>Sounds <b>good</b></div>' });
    const reopened = draftFromMessage({ ...saved, id: d.id });
    expect(reopened.body).toBe('<div>Sounds <b>good</b></div>');
    expect(reopened.quoted).toContain('gmail_quote');
  });
});

describe('sanitisers', () => {
  it('keeps simple formatting and safe links in the editor', () => {
    const out = sanitizeEditorHtml(
      '<p style="color:red" onclick="x()">Hi <b>there</b> <a href="javascript:alert(1)">bad</a> <a href="https://ok.io">ok</a></p><script>x()</script><img src="https://t.io/p.gif">',
    );
    expect(out).toBe('<p>Hi <b>there</b> <a>bad</a> <a href="https://ok.io">ok</a></p>');
  });

  it('keeps styling but drops handlers and scripts in quotes', () => {
    const out = sanitizeQuotedHtml(
      '<td style="color:red" onmouseover="x()">a</td><script>x()</script>',
    );
    expect(out).not.toMatch(/onmouseover|script/);
    expect(sanitizeQuotedHtml('<p style="color:red">a</p>')).toBe('<p style="color:red">a</p>');
  });
});
