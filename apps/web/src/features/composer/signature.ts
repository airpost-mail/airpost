// The sender's default signature (Settings > Signatures), placed below the editor text the
// way the desktop's signature plugin does, when the engine's draft has none of its own. The marker attribute lets an untouched draft
// still count as empty when closed.
import { signatureForEmail } from '@/features/settings';
import type { DraftState } from './draft';
import { sanitizeEditorHtml } from './html';

const ATTR = 'data-airpost-signature';

export function withSignature(draft: DraftState): DraftState {
  const sig = signatureForEmail(draft.from.email);
  return sig ? signed(draft, sig.id, sig.body) : draft;
}

/** The draft with a signature block below its text. */
export function signed(draft: DraftState, id: string, body: string): DraftState {
  const html = sanitizeEditorHtml(body);
  if (!html.trim()) return draft;
  const block = `<div ${ATTR}="${id.replace(/[^\w-]/g, '')}">${html}</div>`;
  return { ...draft, body: `${draft.body}<br><br>${block}` };
}

/** The body without the signature block, for "is this draft empty" checks. */
export function withoutSignature(html: string): string {
  if (!html.includes(ATTR)) return html;
  const doc = new DOMParser().parseFromString(`<!doctype html><body>${html}`, 'text/html');
  for (const el of doc.body.querySelectorAll(`[${ATTR}]`)) el.remove();
  return doc.body.innerHTML;
}
