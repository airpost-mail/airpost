// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import {
  insertIntoHTML,
  insertIntoPlainText,
  insertReply,
  plainTextToHTML,
} from '../lib/reply-insert';
import { convertFromHTML, convertToHTML } from '../../../src/components/composer-editor/conversion';
import {
  deepenPlaintextQuote,
  wrapPlaintext,
} from '../../../src/components/composer-editor/plaintext';

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

  // The body DraftFactory.createDraftForReply builds for a plain text reply.
  const plainReply = (attribution: string, quoted: string) =>
    wrapPlaintext(`\n\n${attribution}\n${deepenPlaintextQuote(quoted)}`);

  it('goes above the attribution line of a plain text reply', () => {
    const body = plainReply('On Tue, Ann wrote:', 'Lunch?');
    expect(body).toBe('\n\nOn Tue, Ann wrote:\n\n> Lunch?');
    expect(insertIntoPlainText(body, 'Yes.\n')).toBe('Yes.\n\nOn Tue, Ann wrote:\n\n> Lunch?');
  });

  it('keeps a soft-wrapped attribution line whole', () => {
    const attribution =
      'On Oct 14 2026, at 9:41 am, Annabel Featherstonehaugh-Whittingham <annabel@example.test> wrote:';
    const body = plainReply(attribution, 'Lunch?');
    expect(body).toContain('\n\u200B');
    const result = insertIntoPlainText(body, 'Yes.');
    expect(result.startsWith('Yes.\n\nOn Oct 14 2026')).toBe(true);
    expect(result.slice('Yes.\n\n'.length)).toBe(body.replace(/^\n+/, ''));
  });

  it('keeps what the user typed above a plain text quote', () => {
    const body = `Thanks!${plainReply('On Tue, Ann wrote:', 'Lunch?')}`;
    expect(insertIntoPlainText(body, 'Noon works.')).toBe(
      'Thanks!\n\nNoon works.\n\nOn Tue, Ann wrote:\n\n> Lunch?'
    );
  });

  it('goes above the quote once the composer has serialized the body', () => {
    const quote = `<blockquote class="gmail_quote" style="margin:0 0 0 .8ex;border-left:1px #ccc solid;padding-left:1ex;"><div>Lunch?</div><div class="gmail_quote">Earlier</div></blockquote>`;
    const serialized = (raw: string) => convertToHTML(convertFromHTML(raw));

    // The user deleted the attribution line: only the classless blockquote is left.
    const bare = serialized(`<div>Thanks!</div>${quote}`);
    expect(bare).not.toContain('<blockquote class=');
    const result = insertIntoHTML(bare, 'Noon works.');
    expect(result.indexOf('Noon works.')).toBeGreaterThan(result.indexOf('Thanks!'));
    expect(result.indexOf('Noon works.')).toBeLessThan(result.indexOf('<blockquote'));

    // An RTL attribution comes out as <div dir="rtl" class="gmail_quote_attribution">.
    const rtl = serialized(
      `<br/><div class="gmail_quote_attribution">في الثلاثاء كتبت آن:</div>${quote}`
    );
    expect(rtl).toContain('dir="rtl"');
    const inserted = insertIntoHTML(rtl, 'Yes.');
    expect(inserted.startsWith('<div>Yes.</div><div><br/></div>')).toBe(true);
    expect(inserted.indexOf('Yes.')).toBeLessThan(inserted.indexOf('gmail_quote_attribution'));
  });

  it('finds an RTL attribution and a bare blockquote', () => {
    const rtl = '<div dir="rtl" class="gmail_quote_attribution">في الثلاثاء كتبت آن:</div>';
    const quote = '<blockquote><div>Lunch?</div></blockquote>';
    expect(insertIntoHTML(`<br>${rtl}${quote}`, 'Yes.')).toBe(
      `<div>Yes.</div><div><br/></div>${rtl}${quote}`
    );
    expect(insertIntoHTML(`<div>Hi</div>${quote}`, 'Yes.')).toBe(
      `<div>Hi</div><div><br/></div><div>Yes.</div><div><br/></div>${quote}`
    );
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
