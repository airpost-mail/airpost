import React from 'react';
import { Editor } from 'slate';
import { RetinaImg } from '../retina-img';
import { localized } from '../../intl/localize';
import SanitizeTransformer from '../../mail/body/sanitize-transformer';
import { SignatureStore } from '../../data/stores/signature';
import { ComposerEditorPlugin, ComposerRenderNodeProps, Rule } from './types';

export const UNEDITABLE_TYPE = 'uneditable';
export const UNEDITABLE_TAGS = ['table', 'img', 'center', 'signature'];

// UneditableNode re-renders on every composer change, but SanitizeTransformer
// has no internal cache and a block's HTML rarely changes, so sanitizing inline
// would run DOMPurify on every keystroke. Memoize by the exact input string —
// the sanitizer is a pure function of (input, static config) — and bound the
// map so a long editing session with many distinct blocks can't grow it without
// limit.
const sanitizeCache = new Map<string, string>();
const SANITIZE_CACHE_MAX = 100;

function sanitizeUneditableHtml(raw: string): string {
  if (!raw) return '';
  const cached = sanitizeCache.get(raw);
  if (cached !== undefined) return cached;
  const clean = SanitizeTransformer.runSync(raw);
  // Map iterates in insertion order, so deleting the first key is FIFO eviction.
  if (sanitizeCache.size >= SANITIZE_CACHE_MAX) {
    const oldest = sanitizeCache.keys().next();
    if (!oldest.done) sanitizeCache.delete(oldest.value);
  }
  sanitizeCache.set(raw, clean);
  return clean;
}

// The composer renders in the app window, whose CSP lets images load from anywhere, so quoted
// mail would fetch its remote images (and trackers) on Reply or Forward even with remote images
// off. While core.reading.autoloadImages is off, the editor shows these blocks without remote
// image URLs, except ones from the user's own signatures. Only the display changes: serialize
// keeps the original HTML, so the sent message still has its images.
const RemoteImageAttributes = ['src', 'background', 'poster'];
const RemoteCSSImageRegexp = /(?:url|image-set)\((?!\s*['"]?\s*(?:data|cid):)[^)]*\)/gi;
const MayHaveRemoteImages = /\b(?:src|srcset|background|poster)\s*=|url\(|image-set\(/i;

function isRemoteImageURL(url: string) {
  return (
    !/^\s*(?:cid|data|airpost-asset|mailspring|airpost-file|file):/i.test(url) && url.trim() !== ''
  );
}

const displayCache = new Map<string, string>();
let signatureURLs: { signatures: unknown; urls: Set<string> } | null = null;

function ownSignatureImageURLs() {
  const signatures = SignatureStore.getSignatures();
  if (!signatureURLs || signatureURLs.signatures !== signatures) {
    const urls = new Set<string>();
    for (const { body } of Object.values(signatures || {})) {
      if (!body) continue;
      const doc = new DOMParser().parseFromString(body, 'text/html');
      // Every element matched [src], so the attribute is present.
      doc.querySelectorAll('[src]').forEach((el) => urls.add(el.getAttribute('src')!));
    }
    signatureURLs = { signatures, urls };
    displayCache.clear();
  }
  return signatureURLs.urls;
}

// DOMParser documents are inert, so parsing here loads nothing.
export function withoutRemoteImages(html: string) {
  if (!MayHaveRemoteImages.test(html)) return html;
  const own = ownSignatureImageURLs();
  const doc = new DOMParser().parseFromString(html, 'text/html');
  doc.body.querySelectorAll('*').forEach((el) => {
    for (const attr of RemoteImageAttributes) {
      const value = el.getAttribute(attr);
      if (value !== null && isRemoteImageURL(value) && !own.has(value)) el.removeAttribute(attr);
    }
    el.removeAttribute('srcset');
    const style = el.getAttribute('style');
    if (style) el.setAttribute('style', style.replace(RemoteCSSImageRegexp, 'none'));
    if (el.tagName === 'STYLE') {
      el.textContent = (el.textContent ?? '').replace(RemoteCSSImageRegexp, 'none');
    }
  });
  return doc.body.innerHTML;
}

function displayHtml(clean: string) {
  if (AppEnv.config.get('core.reading.autoloadImages')) return clean;
  ownSignatureImageURLs();
  let display = displayCache.get(clean);
  if (display === undefined) {
    display = withoutRemoteImages(clean);
    if (displayCache.size >= SANITIZE_CACHE_MAX) {
      const oldest = displayCache.keys().next();
      if (!oldest.done) displayCache.delete(oldest.value);
    }
    displayCache.set(clean, display);
  }
  return display;
}

function UneditableNode(props: ComposerRenderNodeProps) {
  const { attributes, node, editor, targetIsHTML, isFocused, children } = props;
  // Sanitize at the rendering boundary rather than trusting data.html. Not every
  // path that produces an uneditable block runs it through the HTML deserializer
  // that would otherwise clean this value, so sanitize here before it reaches the
  // DOM. The result is memoized, so an already-clean value costs a map lookup
  // rather than a DOMPurify run. See GHSA-2x8h-f5qm-f779.
  const rawHtml = node.data.get('html');
  const __html = sanitizeUneditableHtml(rawHtml);

  if (targetIsHTML) {
    return <div dangerouslySetInnerHTML={{ __html }} />;
  }
  return (
    <div {...attributes} className={`uneditable custom-block ${isFocused && 'focused'}`}>
      <a
        className="uneditable-remove"
        onClick={(e) => {
          e.stopPropagation();
          e.preventDefault();
          // slate-react always passes the editor when rendering into it
          editor!.removeNodeByKey(node.key);
        }}
      >
        <RetinaImg
          title={localized('Remove HTML')}
          name="image-cancel-button.png"
          mode={RetinaImg.Mode.ContentPreserve}
        />
      </a>
      <div dangerouslySetInnerHTML={{ __html: displayHtml(__html) }} />
      {/* This node below is necessary for selection of the uneditable block, and
      we also put the text content of the HTML in so that copy/paste works nicely. */}
      <div style={{ position: 'absolute', height: 0, overflow: 'hidden' }}>{children}</div>
    </div>
  );
}

function renderNode(
  props: ComposerRenderNodeProps,
  editor: Editor | null = null,
  next: () => void = () => {}
) {
  if (props.node.type === UNEDITABLE_TYPE) {
    return UneditableNode(props);
  }
  return next();
}

const rules: Rule[] = [
  {
    deserialize(el, next) {
      const tagName = el.tagName.toLowerCase();

      if (UNEDITABLE_TAGS.includes(tagName)) {
        // The captured HTML is later re-injected via dangerouslySetInnerHTML in
        // UneditableNode. Because this composer runs in a renderer with nodeIntegration,
        // any preserved <img onerror=...>, <iframe>, or <script> would execute with Node
        // access. Sanitize here so every input path (mailto, paste, drag-drop, programmatic)
        // is covered at the funnel.
        return {
          object: 'block',
          type: UNEDITABLE_TYPE,
          data: {
            html: SanitizeTransformer.runSync(el.outerHTML),
          },
          nodes: [],
        };
      }
    },
    serialize(obj: any, children: any) {
      if (obj.object !== 'block') return;
      return renderNode({ node: obj, children, targetIsHTML: true });
    },
  },
];

const plugins: ComposerEditorPlugin[] = [
  {
    renderNode,
    rules,
  },
];

export default plugins;
