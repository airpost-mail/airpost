// Puts a drafted reply into the composer's body: above the signature and anything the user
// typed below it, as one edit the editor can undo. The quoted original is outside the
// editor, so it stays below by itself.
import type { BodyEditorHandle } from '@/features/composer/BodyEditor';

const SIGNATURE = '[data-tern-signature]';

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/** Model text as editor HTML: escaped, line breaks kept, nothing else. */
export function replyHtml(text: string): string {
  return escapeHtml(text.trim()).replace(/\r?\n/g, '<br>');
}

function isBlank(node: Node): boolean {
  if (node.nodeType === Node.TEXT_NODE) return !node.textContent?.trim();
  if (node.nodeType !== Node.ELEMENT_NODE) return true;
  const el = node as Element;
  return el.tagName === 'BR' || (!el.textContent?.trim() && !el.querySelector('img'));
}

/**
 * Where the reply goes: the start of the body when nothing was typed above the signature,
 * else right after what was typed.
 */
function insertionPoint(body: HTMLElement): { range: Range; typed: boolean } {
  const range = document.createRange();
  const sig = [...body.children].find((c) => c.matches(SIGNATURE) || c.querySelector(SIGNATURE));
  const above = [...body.childNodes];
  const end = sig ? above.indexOf(sig) : above.length;
  let last = end - 1;
  while (last >= 0 && isBlank(above[last])) last--;
  if (last < 0) {
    range.setStart(body, 0);
    return { range, typed: false };
  }
  range.setStartAfter(above[last]);
  return { range, typed: true };
}

export function insertReply(editor: BodyEditorHandle, text: string) {
  const body = editor.element();
  const html = replyHtml(text);
  if (!body || !html) return;
  const { range, typed } = insertionPoint(body);
  range.collapse(true);
  const sel = window.getSelection();
  sel?.removeAllRanges();
  sel?.addRange(range);
  editor.saveSelection();
  editor.insertHtml(typed ? `<br><br>${html}` : html);
}
