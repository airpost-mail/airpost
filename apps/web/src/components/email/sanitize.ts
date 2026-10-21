// Prepares an HTML body for the sandboxed frame. The sandbox (no allow-scripts) and the CSP
// do the real work; this only removes what they cannot stop and rewrites cid: references.
import { GATEWAY_BASE } from '@/engine/mode';

const REMOTE_RE =
  /\b(?:src|background|poster|srcset)\s*=\s*["']?\s*(?:https?:)?\/\/|url\(\s*["']?\s*(?:https?:)?\/\//i;

export function hasRemoteContent(html: string): boolean {
  return REMOTE_RE.test(html);
}

/**
 * CSP for the frame document. Remote images are opt-in per message. Inline parts load from
 * the gateway's file route (`cid:` rewritten), and only from
 * there: the frame is same-origin (`allow-same-origin`), so the pairing cookie goes along
 * and `'self'` would let a message's `<img>` hit any gateway GET.
 */
export function frameCsp(allowRemoteImages: boolean): string {
  const files = `${location.origin}${GATEWAY_BASE}/files/`;
  const img = allowRemoteImages ? `${files} data: blob: https: http:` : `${files} data: blob:`;
  return [
    "default-src 'none'",
    `img-src ${img}`,
    "style-src 'unsafe-inline'",
    'font-src data:',
    'media-src data: blob:',
    "form-action 'none'",
  ].join('; ');
}

/**
 * Elements the frame must never see. `meta[http-equiv]` covers refresh (navigates the frame)
 * and x-dns-prefetch-control; `link` covers preconnect, dns-prefetch and prefetch, which no
 * CSP directive governs, so they would reach a tracker even with images blocked. `base`
 * would retarget relative URLs. Embedded documents and plugins load their own content.
 * Scripts never run (no allow-scripts); dropping them keeps the DOM small.
 */
const STRIP = [
  'script',
  'base',
  'link',
  'meta[http-equiv]',
  'iframe',
  'frame',
  'frameset',
  'object',
  'embed',
  'applet',
  'portal',
].join(',');

export function prepareHtml(html: string, resolveCid?: (cid: string) => string | null): string {
  // DOMParser documents are inert: nothing in them loads or runs while we clean it.
  const doc = new DOMParser().parseFromString(html, 'text/html');
  for (const el of [...doc.querySelectorAll(STRIP)]) el.remove();
  for (const el of [...doc.querySelectorAll('[ping]')]) el.removeAttribute('ping');
  // Head styles still apply from the body; the frame supplies its own head.
  let out = doc.head.innerHTML + doc.body.innerHTML;
  if (resolveCid) {
    out = out.replace(/(["'(])cid:([^"')\s]+)/gi, (m, q: string, cid: string) => {
      const url = resolveCid(decodeURIComponent(cid));
      return url ? `${q}${url}` : m;
    });
  }
  return out;
}

export function buildSrcDoc(body: string, opts: { allowRemoteImages: boolean }): string {
  return `<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${frameCsp(opts.allowRemoteImages)}">
<meta http-equiv="x-dns-prefetch-control" content="off">
<meta name="color-scheme" content="light">
<style>
html,body{margin:0;padding:0;background:#fff}
body{font:14px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI",Helvetica,Arial,sans-serif;color:#1d2433;overflow-wrap:anywhere;padding:2px}
img{max-width:100%;height:auto}
table{max-width:100%}
pre{white-space:pre-wrap}
blockquote{margin:0 0 0 .8ex;border-left:2px solid #d4d8e1;padding-left:1ex;color:#59606f}
a{color:#2a5bd7}
</style></head><body>${body}</body></html>`;
}
