// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Puts a drafted reply into a composer's body above the quoted text (and the signature), as one
body change so a single undo takes it out again. The model's text is plain: it is escaped, never
parsed as HTML (docs/ai-plan.md A6).
*/

// Where the quote or the signature starts in an HTML reply.
const HTML_MARKERS = [
  '<signature',
  '<div class="gmail_quote_attribution"',
  '<blockquote class="gmail_quote"',
  '<div class="gmail_quote"',
];

export function escapeHTML(text: string) {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/** Paragraphs as divs, line breaks as <br/>, a blank div between paragraphs. */
export function plainTextToHTML(text: string) {
  const paragraphs = text
    .replace(/\r\n?/g, '\n')
    .trim()
    .split(/\n\s*\n/)
    .map((p) => p.replace(/^\n+|\n+$/g, ''))
    .filter((p) => p.length > 0);
  return paragraphs
    .map((p) => `<div>${p.split('\n').map(escapeHTML).join('<br/>')}</div>`)
    .join('<div><br/></div>');
}

// Whether a piece of HTML shows nothing but blank space.
function isBlankHTML(html: string) {
  return (
    html
      .replace(/<[^>]*>/g, '')
      .replace(/&nbsp;/g, '')
      .replace(/\u200B/g, '')
      .trim() === ''
  );
}

export function insertIntoHTML(body: string, text: string) {
  let insertion = body.length;
  for (const marker of HTML_MARKERS) {
    const i = body.indexOf(marker);
    if (i !== -1) insertion = Math.min(insertion, i);
  }
  const before = body.slice(0, insertion);
  const after = body.slice(insertion);
  const reply = plainTextToHTML(text);
  const kept = isBlankHTML(before) ? '' : `${before}<div><br/></div>`;
  return `${kept}${reply}<div><br/></div>${after}`;
}

/** The start of the quote in a plain text reply: the line before the first `>` line. */
function plainQuoteStart(body: string) {
  const lines = body.split('\n');
  let offset = 0;
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].startsWith('>')) {
      if (i > 0 && lines[i - 1].trim() !== '') return offset - lines[i - 1].length - 1;
      return offset;
    }
    offset += lines[i].length + 1;
  }
  return body.length;
}

export function insertIntoPlainText(body: string, text: string) {
  const insertion = Math.max(0, plainQuoteStart(body));
  const before = body.slice(0, insertion);
  const after = body.slice(insertion);
  const reply = text.replace(/\r\n?/g, '\n').trim();
  const kept = before.replace(/\u200B/g, '').trim() === '' ? '' : `${before.trimEnd()}\n\n`;
  return `${kept}${reply}\n\n${after}`;
}

export interface ReplySession {
  draft(): { body?: string; plaintext?: boolean };
  changes: { add(changes: { body: string }): void };
}

export function insertReply(session: ReplySession, text: string) {
  const draft = session.draft();
  const body = draft.body || '';
  session.changes.add({
    body: draft.plaintext ? insertIntoPlainText(body, text) : insertIntoHTML(body, text),
  });
}
