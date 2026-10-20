// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Puts a drafted reply into a composer's body above the quoted text (and the signature), as one
body change so a single undo takes it out again. The model's text is plain: it is escaped, never
parsed as HTML (docs/ai-plan.md A6).
*/

// Where the quote or the signature starts in an HTML reply, by element: the composer's serializer
// drops the blockquote's class and may put `dir` before `class`, so string prefixes don't hold.
// Any blockquote counts, as in the composer's own hasBlockquote.
const QUOTE_SELECTOR = 'signature, .gmail_quote_attribution, .gmail_quote, blockquote';

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

/** The body split before its first top-level node that is or holds the quote or the signature. */
function splitHTML(body: string) {
  // A DOMParser document is inert: nothing in the quoted mail loads or runs.
  const doc = new DOMParser().parseFromString(body, 'text/html');
  const nodes = Array.from(doc.body.childNodes);
  let index = nodes.findIndex(
    (n) =>
      n.nodeType === Node.ELEMENT_NODE &&
      ((n as Element).matches(QUOTE_SELECTOR) || !!(n as Element).querySelector(QUOTE_SELECTOR))
  );
  if (index === -1) index = nodes.length;
  const before = doc.createElement('div');
  for (const n of nodes.slice(0, index)) before.appendChild(n);
  return { before: before.innerHTML, after: doc.body.innerHTML };
}

export function insertIntoHTML(body: string, text: string) {
  const { before, after } = splitHTML(body);
  const reply = plainTextToHTML(text);
  const kept = isBlankHTML(before) ? '' : `${before}<div><br/></div>`;
  return `${kept}${reply}<div><br/></div>${after}`;
}

const isBlank = (line: string) => line.replace(/\u200B/g, '').trim() === '';

/**
 * The start of the quote in a plain text reply: the attribution line above the first `>` line.
 * DraftFactory leaves a blank line between them, and wrapPlaintext may have soft-wrapped the
 * attribution into lines that start with a zero-width space (SOFT_NEWLINE).
 */
function plainQuoteStart(body: string) {
  const lines = body.split('\n');
  const starts: number[] = [];
  let offset = 0;
  for (const line of lines) {
    starts.push(offset);
    offset += line.length + 1;
  }
  const quote = lines.findIndex((line) => line.startsWith('>'));
  if (quote === -1) return body.length;
  let i = quote;
  while (i > 0 && isBlank(lines[i - 1])) i--;
  if (i === 0) return starts[quote];
  i--;
  while (i > 0 && lines[i].startsWith('\u200B')) i--;
  return starts[i];
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
