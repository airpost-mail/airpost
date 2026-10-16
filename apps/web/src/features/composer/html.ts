// HTML handling for the editor. Anything that reaches the contentEditable (pastes, reopened
// drafts) goes through `sanitizeEditorHtml` first, because the editor is live DOM in the app
// origin: an `onerror` attribute there would run. Quoted originals never enter the editor;
// they are previewed in the sandboxed frame and appended at save time.

const EDITOR_TAGS = new Set([
  'a',
  'b',
  'blockquote',
  'br',
  'code',
  'div',
  'em',
  'h1',
  'h2',
  'h3',
  'h4',
  'hr',
  'i',
  'li',
  'ol',
  'p',
  'pre',
  's',
  'span',
  'strike',
  'strong',
  'sub',
  'sup',
  'table',
  'tbody',
  'td',
  'th',
  'thead',
  'tr',
  'u',
  'ul',
]);

/** Dropped with their content. Everything else unknown is unwrapped. */
const DROP_TAGS = new Set([
  'script',
  'style',
  'iframe',
  'object',
  'embed',
  'link',
  'meta',
  'base',
  'form',
  'input',
  'button',
  'select',
  'textarea',
  'template',
  'svg',
  'math',
  'noscript',
  'title',
  'head',
]);

const SAFE_URL_RE = /^(?:https?:|mailto:)/i;

export function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

export function textToHtml(text: string): string {
  return escapeHtml(text).replace(/\r?\n/g, '<br>');
}

function parse(html: string): Document {
  // DOMParser documents are inert: nothing loads and no handlers run.
  return new DOMParser().parseFromString(`<!doctype html><body>${html}`, 'text/html');
}

function cleanEditorNode(node: Node, doc: Document) {
  for (const child of [...node.childNodes]) {
    if (child.nodeType === Node.COMMENT_NODE) {
      child.remove();
      continue;
    }
    if (child.nodeType !== Node.ELEMENT_NODE) continue;
    const el = child as Element;
    const tag = el.tagName.toLowerCase();
    if (DROP_TAGS.has(tag)) {
      el.remove();
      continue;
    }
    cleanEditorNode(el, doc);
    if (!EDITOR_TAGS.has(tag)) {
      el.replaceWith(...el.childNodes);
      continue;
    }
    for (const attr of [...el.attributes]) {
      const name = attr.name.toLowerCase();
      const keep =
        (tag === 'a' && name === 'href' && SAFE_URL_RE.test(attr.value.trim())) ||
        (name === 'dir' && /^(ltr|rtl|auto)$/i.test(attr.value)) ||
        ((tag === 'td' || tag === 'th') && (name === 'colspan' || name === 'rowspan'));
      if (!keep) el.removeAttribute(attr.name);
    }
  }
}

/** Allow-list sanitiser for HTML that goes into the live editor. */
export function sanitizeEditorHtml(html: string): string {
  const doc = parse(html);
  cleanEditorNode(doc.body, doc);
  return doc.body.innerHTML;
}

/**
 * Light clean-up for a quoted original. It keeps the sender's styling because the result is
 * only ever shown in the sandboxed frame and sent on; it removes active content.
 */
export function sanitizeQuotedHtml(html: string): string {
  const doc = parse(html);
  for (const el of [...doc.body.querySelectorAll('*')]) {
    const tag = el.tagName.toLowerCase();
    if (DROP_TAGS.has(tag) && tag !== 'style') {
      el.remove();
      continue;
    }
    for (const attr of [...el.attributes]) {
      const name = attr.name.toLowerCase();
      const value = attr.value.trim();
      if (name.startsWith('on')) el.removeAttribute(attr.name);
      else if (
        /^(href|src|action|formaction|xlink:href)$/.test(name) &&
        /^javascript:/i.test(value)
      )
        el.removeAttribute(attr.name);
    }
  }
  return doc.body.innerHTML;
}

export function htmlToText(html: string): string {
  const doc = parse(html.replace(/<br\s*\/?>/gi, '\n'));
  for (const el of [...doc.body.querySelectorAll('style,script')]) el.remove();
  return (doc.body.textContent ?? '').replace(/\u00a0/g, ' ');
}

export function isBlankHtml(html: string): boolean {
  return htmlToText(html).trim() === '' && !/<(img|hr|table)\b/i.test(html);
}

const QUOTE_MARKERS = '.gmail_quote_attribution, .gmail_quote, blockquote[type="cite"]';

/**
 * Splits a saved draft body into what the user wrote and the quoted original, so a reopened
 * reply edits the same way a fresh one does. Only top-level markers count: a marker nested
 * in the user's own text stays editable.
 */
export function splitQuoted(html: string): { editor: string; quoted: string | null } {
  const doc = parse(html);
  const children = [...doc.body.childNodes];
  const index = children.findIndex(
    (n) => n.nodeType === Node.ELEMENT_NODE && (n as Element).matches(QUOTE_MARKERS),
  );
  if (index === -1) return { editor: sanitizeEditorHtml(html), quoted: null };
  const wrap = (nodes: ChildNode[]) => {
    const div = doc.createElement('div');
    div.append(...nodes.map((n) => n.cloneNode(true)));
    return div.innerHTML;
  };
  const editor = wrap(children.slice(0, index)).replace(/(?:\s|<br\s*\/?>)+$/i, '');
  return {
    editor: sanitizeEditorHtml(editor),
    quoted: sanitizeQuotedHtml(wrap(children.slice(index))),
  };
}

/** Accepts what people type into a link box: full URLs, bare domains, email addresses. */
export function normalizeUrl(input: string): string | null {
  const s = input.trim();
  if (!s) return null;
  if (/^(https?:|mailto:)/i.test(s)) return s;
  if (/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(s)) return `mailto:${s}`;
  if (/^[\w-]+(\.[\w-]+)+(\/\S*)?$/.test(s)) return `https://${s}`;
  return null;
}
