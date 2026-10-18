// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import {
  insertIntoHTML,
  insertIntoPlainText,
  insertReply,
  plainTextToHTML,
} from '../lib/reply-insert';

const QUOTE =
  '<div class="gmail_quote_attribution">On Tue, Ann wrote:</div><blockquote class="gmail_quote"><p>Lunch?</p></blockquote>';

describe('Inserting a drafted reply', () => {
  it('escapes the model text and keeps its paragraphs and lines', () => {
    expect(plainTextToHTML('Hi <Ann>,\n\nSure & see you.\nBob')).toBe(
      '<div>Hi &lt;Ann&gt;,</div><div><br/></div><div>Sure &amp; see you.<br/>Bob</div>'
    );
    expect(plainTextToHTML('<img src="https://x.test/p.png">')).not.toContain('<img');
  });

  it('replaces the blank lines of a fresh reply and keeps the quote below', () => {
    const body = `<br/><br/>${QUOTE}`;
    expect(insertIntoHTML(body, 'Yes, noon works.')).toBe(
      `<div>Yes, noon works.</div><div><br/></div>${QUOTE}`
    );
  });

  it('keeps what the user typed and goes above the signature', () => {
    const signature = '<signature id="s1"><div>Bob</div></signature>';
    const body = `<div>Thanks!</div>${signature}${QUOTE}`;
    expect(insertIntoHTML(body, 'Noon works.')).toBe(
      `<div>Thanks!</div><div><br/></div><div>Noon works.</div><div><br/></div>${signature}${QUOTE}`
    );
  });

  it('goes above the attribution line of a plain text reply', () => {
    const body = '\n\nOn Tue, Ann wrote:\n> Lunch?\n';
    expect(insertIntoPlainText(body, 'Yes.\n')).toBe('Yes.\n\nOn Tue, Ann wrote:\n> Lunch?\n');
  });

  it('is one body change on the session', () => {
    const added: any[] = [];
    const session = {
      draft: () => ({ body: `<br/>${QUOTE}`, plaintext: false }),
      changes: { add: (c: any) => added.push(c) },
    };
    insertReply(session, 'Sure.');
    expect(added).toEqual([{ body: `<div>Sure.</div><div><br/></div>${QUOTE}` }]);
  });
});
