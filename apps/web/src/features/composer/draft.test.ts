import { describe, expect, it } from 'vitest';
import type { MessageJSON } from '@/engine';
import { draftFromMessage, draftToMessage, fullBody } from './draft';
import { sanitizeEditorHtml, sanitizeQuotedHtml } from './html';

const me = { name: 'Maya', email: 'maya@okafor.studio' };

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

describe('draft bodies', () => {
  // A reply as `draft.create` builds it: the user's part, then the quote.
  const reply = message({
    id: 'd1',
    draft: true,
    hMsgId: 'reply@x.io',
    rthMsgId: 'orig@x.io',
    from: [me],
    to: [{ name: 'Ann', email: 'ann@x.io' }],
    body: [
      '<br><br><div class="gmail_quote_attribution">On Jun 1, Ann wrote:</div>',
      '<blockquote class="gmail_quote"><p>Hello <img src=x onerror="alert(1)"></p></blockquote>',
    ].join(''),
  });

  it('keeps the quote out of the editor, without active content, and threads the reply', () => {
    const d = draftFromMessage(reply);
    expect(d.body).toBe('');
    expect(d.quoted).toContain('gmail_quote');
    expect(d.quoted).not.toContain('onerror');
    const m = draftToMessage({ ...d, body: '<div>Sounds good</div>' });
    expect(m).toMatchObject({ rthMsgId: 'orig@x.io', threadId: 't1', draft: true, aid: 'acc' });
    expect(m.body).toMatch(/^<div>Sounds good<\/div><br><br><div class="gmail_quote_attribution">/);
  });

  it('leaves the quote out when it is switched off', () => {
    const d = { ...draftFromMessage(reply), body: '<div>Hi</div>', includeQuoted: false };
    expect(fullBody(d)).toBe('<div>Hi</div>');
  });

  it('reopens a saved reply with the quote split off', () => {
    const d = draftFromMessage(reply);
    const saved = draftToMessage({ ...d, body: '<div>Sounds <b>good</b></div>' });
    const reopened = draftFromMessage(saved);
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
