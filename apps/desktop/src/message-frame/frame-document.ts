// The DOM work done inside the message frame: writing the body, its colours, links and
// images, measuring it, find-in-thread highlights and the operations extensions run on it.
// Everything here takes the document to work on, so specs can run it on a same-origin
// iframe; frame-runtime.ts wires it to postMessage inside the real frame.
import { applyDocumentColorScheme, darkenDocument, removeDarkening } from '@tern/email-dark';
import { Autolink } from '../services/autolinker';
import RealDOMParser from '../searchable-components/real-dom-parser';
import { DOMUtils } from './frame-exports';
import { adjustImages } from './adjust-images';
import { prepareEmailColors } from './email-color-detection';
import { BodyOperation, FrameRect, RenderMessage, SearchMatchRect } from './message-frame-protocol';

export const ThemeStylesId = 'tern-theme-styles';
export const HTMLWrapperId = 'inbox-html-wrapper';
export const PlainWrapperId = 'inbox-plain-wrapper';

// A height past this is a layout loop, not an email.
const MaxHeight = 200000;

function themeStyleElement(doc: Document) {
  let style = doc.getElementById(ThemeStylesId) as HTMLStyleElement | null;
  if (!style) {
    style = doc.createElement('style');
    style.id = ThemeStylesId;
    (doc.head || doc.documentElement).appendChild(style);
  }
  return style;
}

/** Replaces the frame's body with the message. */
export function renderMessageDocument(doc: Document, msg: RenderMessage) {
  removeDarkening(doc);
  themeStyleElement(doc).textContent = msg.themeStyles || '';

  const wrapper = doc.createElement('div');
  wrapper.className = msg.platform;
  wrapper.setAttribute('role', 'document');
  if (msg.plaintext) {
    wrapper.id = PlainWrapperId;
    wrapper.innerText = msg.html;
  } else {
    wrapper.id = HTMLWrapperId;
    wrapper.innerHTML = msg.html;
  }
  doc.body.replaceChildren(wrapper);
  doc.body.classList.toggle('restrict-width', !!msg.restrictWidth);

  // Before darkening, so an email's own `prefers-color-scheme: dark` rules are what it sees.
  applyDocumentColorScheme(doc, msg.colorScheme);
  if (msg.plaintext) return wrapper;

  try {
    if (msg.darkPalette) {
      darkenDocument(doc, msg.darkPalette, { root: wrapper, imageBrightness: 0.9 });
    } else {
      const { r, g, b, a } = msg.themeBackground;
      wrapper.classList.toggle('has-background', prepareEmailColors(wrapper, [r, g, b, a]));
    }
  } catch (e) {
    console.error(e);
  }
  return wrapper;
}

/** Makes link-like text clickable and keeps images inside the frame's width. */
export function decorateMessageDocument(doc: Document, { async }: { async: boolean }) {
  if (!doc.body) return;
  Autolink(doc.body, { async, telAggressiveMatch: false });
  adjustImages(doc);
}

/**
 * The height the frame needs to show the whole body without a vertical scrollbar. Measured
 * with `html` and `body` forced to auto height, so a body styled `height: 100%` reports its
 * content rather than the frame's current height and the frame can shrink.
 */
export function measureMessageDocument(doc: Document) {
  const html = doc.documentElement;
  const win = doc.defaultView;
  if (!html || !win) return 0;

  const probe = doc.createElement('style');
  probe.textContent = 'html,body{height:auto!important;min-height:0!important}';
  (doc.head || html).appendChild(probe);
  let height = Math.ceil(html.getBoundingClientRect().height);
  if (html.scrollHeight > win.innerHeight) {
    height = Math.max(height, html.scrollHeight);
  }
  const scrollsHorizontally = html.scrollWidth > html.clientWidth;
  probe.remove();

  // Room for the horizontal scrollbar, which otherwise covers the last line. Content exactly
  // as tall as the frame is sized by the frame (vh units), and adding to it would never end.
  if (scrollsHorizontally && height !== win.innerHeight) {
    height += 20;
  }
  return Math.max(0, Math.min(height, MaxHeight));
}

function frameRect(el: Element): FrameRect {
  const r = el.getBoundingClientRect();
  return { top: r.top, left: r.left, width: r.width, height: r.height };
}

/**
 * Highlights `term` in the body (the match at `currentIndex` gets `current-match`) and
 * returns the visible matches' boxes in frame coordinates, in render order.
 */
export function searchMessageDocument(
  doc: Document,
  regionId: string,
  term: string | null,
  currentIndex: number | null
): SearchMatchRect[] {
  const root = doc.body || doc.documentElement;
  if (!root) return [];
  const parser = new RealDOMParser(regionId);
  if (term && parser.matchesSearch(root, term)) {
    parser.removeMatchesAndNormalize(root);
    const matchNodeMap = parser.getElementsWithNewMatchNodes(root, term, currentIndex);
    parser.highlightSearch(root, matchNodeMap);
  } else {
    parser.removeMatchesAndNormalize(root);
    return [];
  }
  return Array.from(root.querySelectorAll<HTMLElement>('search-match'))
    .filter((node) => DOMUtils.nodeIsLikelyVisible(node))
    .map((node) => ({ ...frameRect(node), renderIndex: +node.getAttribute('data-render-index') }))
    .sort((a, b) => a.renderIndex - b.renderIndex);
}

/** The body as printed (the sender's colours, no frame scripts) and its text. */
export function messageDocumentContent(doc: Document) {
  const root = doc.documentElement.cloneNode(true) as HTMLElement;
  removeDarkening(root);
  for (const el of Array.from(root.querySelectorAll('script'))) el.remove();
  return { html: root.innerHTML, text: doc.body ? doc.body.innerText : '' };
}

export interface BodyElementInfo {
  ref: number;
  tagName: string;
  attributes: { [name: string]: string };
  text: string;
}

// Elements extensions may not add: anything that runs, loads a document or navigates.
const RefusedTags = new Set([
  'script',
  'iframe',
  'frame',
  'object',
  'embed',
  'meta',
  'link',
  'base',
  'form',
  'style',
]);

function checkAttributeName(name: string) {
  if (!/^[a-z][a-z0-9_:.-]*$/i.test(name) || /^on/i.test(name)) {
    throw new Error(`attribute ${name} is not allowed`);
  }
}

/**
 * The elements extensions reach through `MessageBody` (src/message-frame/message-body.ts),
 * by number. Numbers stay valid until the next render.
 */
export class BodyElements {
  _refs = new Map<number, Element>();
  _byElement = new Map<Element, number>();
  _next = 1;

  constructor(public doc: Document) {}

  reset() {
    this._refs.clear();
    this._byElement.clear();
  }

  refFor(el: Element) {
    let ref = this._byElement.get(el);
    if (!ref) {
      ref = this._next++;
      this._refs.set(ref, el);
      this._byElement.set(el, ref);
    }
    return ref;
  }

  element(ref: number) {
    const el = this._refs.get(ref);
    if (!el || !el.isConnected) throw new Error(`no element ${ref}`);
    return el;
  }

  info(el: Element): BodyElementInfo {
    const attributes = {};
    for (const attr of Array.from(el.attributes)) attributes[attr.name] = attr.value;
    return {
      ref: this.refFor(el),
      tagName: el.tagName,
      attributes,
      text: (el as HTMLElement).innerText || el.textContent || '',
    };
  }

  /**
   * Runs one operation. `listen` hands the listener to `onEvent`, which the runtime turns
   * into an `event` message.
   */
  run(op: BodyOperation, onEvent?: (listenerId: number, rect: FrameRect) => void): unknown {
    switch (op.op) {
      case 'content':
        return messageDocumentContent(this.doc);
      case 'query': {
        const root = this.doc.body;
        if (!root) return [];
        return Array.from(root.querySelectorAll(op.selector)).map((el) => this.info(el));
      }
      case 'setAttributes': {
        const el = this.element(op.ref);
        for (const [name, value] of Object.entries(op.attributes || {})) {
          checkAttributeName(name);
          if (value === null) el.removeAttribute(name);
          else el.setAttribute(name, String(value));
        }
        return null;
      }
      case 'insert': {
        const tagName = String(op.tagName || '').toLowerCase();
        if (!/^[a-z][a-z0-9-]*$/.test(tagName) || RefusedTags.has(tagName)) {
          throw new Error(`element ${op.tagName} is not allowed`);
        }
        if (!['beforebegin', 'afterbegin', 'beforeend', 'afterend'].includes(op.position)) {
          throw new Error(`position ${op.position} is not allowed`);
        }
        const target = this.element(op.ref);
        const el = this.doc.createElement(tagName);
        for (const [name, value] of Object.entries(op.attributes || {})) {
          checkAttributeName(name);
          el.setAttribute(name, String(value));
        }
        if (op.text) el.textContent = op.text;
        target.insertAdjacentElement(op.position, el);
        return this.refFor(el);
      }
      case 'rect':
        return frameRect(this.element(op.ref));
      case 'listen': {
        if (op.event !== 'click' && op.event !== 'mousedown') {
          throw new Error(`event ${op.event} is not allowed`);
        }
        const el = this.element(op.ref);
        el.addEventListener(op.event, (e) => {
          e.preventDefault();
          e.stopPropagation();
          if (onEvent) onEvent(op.listenerId, frameRect(el));
        });
        return null;
      }
      default:
        throw new Error(`unknown operation ${(op as any).op}`);
    }
  }
}
