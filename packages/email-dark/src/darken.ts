// SPDX-License-Identifier: GPL-3.0-only
//
// Darkens an HTML email in place, in the manner of Dark Reader's dynamic mode: every computed
// background, text, border and SVG paint colour is mapped into a dark scheme built from the
// target palette (see color.ts). It never runs or inserts script: it reads computed styles from
// the parent realm, tags elements with a rule id and adds one <style> element.
//
// The pass reads everything first and writes once at the end, so the document is laid out at
// most once. Results are rules keyed by declaration set, so a newsletter with thousands of cells
// in a handful of colours produces a handful of rules. `removeDarkening` undoes all of it, also
// on a cloned tree (used for printing).

import { RGBA, ColorMapper, composite, formatColor, parseColor, polesFor } from './color';
import { PaintedRegion, isAlreadyDark } from './detect';

export interface Palette {
  // CSS colours: hex, rgb() or rgba().
  background: string;
  text: string;
  link: string;
}

export interface DarkenOptions {
  // Subtree to darken; defaults to the document body. Its own background and colour are set to
  // the palette.
  root?: Element;
  // Past this many elements the email gets a flat palette override instead of per-element
  // mapping.
  maxElements?: number;
  // Below 1, images and videos are dimmed by this brightness factor.
  imageBrightness?: number;
  // Copying a selection serialises computed colours into the clipboard; with this set the
  // darkening is switched off for the duration of the copy, so pasted text keeps the sender's
  // colours.
  restoreOnCopy?: boolean;
}

export type DarkenStatus = 'darkened' | 'already-dark' | 'fallback';

export interface DarkenResult {
  status: DarkenStatus;
  // Elements inspected.
  elements: number;
}

export const DEFAULT_MAX_ELEMENTS = 5000;

const ROOT_ATTR = 'data-email-dark-root';
const MARK_ATTR = 'data-email-dark';
const SAVED_STYLE_ATTR = 'data-email-dark-style';
const SHEET_ATTR = 'data-email-dark-sheet';
// Three id-level points of specificity, so the rules beat `!important` declarations in the
// email's own <style> blocks. `:not(#_)` matches any element not literally id'd "_".
const BOOST = ':not(#_):not(#_):not(#_)';
const SVG_NS = 'http://www.w3.org/2000/svg';

const SKIP_TAGS = new Set([
  'script',
  'style',
  'template',
  'noscript',
  'title',
  'meta',
  'link',
  'base',
  'head',
]);
const MEDIA_TAGS = new Set([
  'img',
  'picture',
  'video',
  'audio',
  'canvas',
  'iframe',
  'object',
  'embed',
  'image',
]);
const COLOR_TAGS = new Set(['a', 'input', 'textarea', 'select', 'button', 'li']);
const SIDES = ['top', 'right', 'bottom', 'left'];
const WHITE: RGBA = { r: 255, g: 255, b: 255, a: 1 };

type Decl = [property: string, value: string];

interface ElementRecord {
  el: Element;
  decls: Decl[];
  // The element has an inline `!important` on a property we change; only an inline
  // `!important` can override it.
  inline: boolean;
}

interface Frame {
  el: Element;
  // Inside a background image or gradient: colours stay as the sender set them.
  untouched: boolean;
  region: number;
  regionColor: RGBA;
}

const copyListeners = new WeakMap<Document, () => void>();

function hasOwnText(el: Element) {
  for (let n = el.firstChild; n; n = n.nextSibling) {
    if (n.nodeType === 3 && /\S/.test((n as Text).data)) return true;
  }
  return false;
}

function parsePalette(palette: Palette) {
  const background = parseColor(palette.background);
  const text = parseColor(palette.text);
  const link = parseColor(palette.link);
  if (!background || !text || !link) {
    throw new Error(`email-dark: unparseable palette ${JSON.stringify(palette)}`);
  }
  return {
    background: composite(background, WHITE),
    text: composite(text, WHITE),
    link: composite(link, WHITE),
  };
}

// The colour the email sits on, as the sender intended it. Hosts often force html/body to be
// transparent, which drops `<body bgcolor>`, so that attribute is consulted too.
function pageColor(root: Element, win: Window): RGBA {
  for (let n: Element | null = root; n; n = n.parentElement) {
    const c = parseColor(win.getComputedStyle(n).backgroundColor);
    if (c && c.a >= 0.5) return composite(c, WHITE);
  }
  const doc = root.ownerDocument;
  const attr = doc.body && doc.body.getAttribute('bgcolor');
  if (attr) {
    let c = parseColor(attr);
    if (!c) {
      const probe = doc.createElement('span');
      probe.style.color = attr;
      root.appendChild(probe);
      c = parseColor(win.getComputedStyle(probe).color);
      probe.remove();
    }
    if (c && c.a > 0) return composite(c, WHITE);
  }
  return WHITE;
}

function hasInlineImportant(el: Element, props: string[]) {
  if (!el.hasAttribute('style')) return false;
  const style = (el as HTMLElement).style;
  if (!style) return false;
  return props.some((p) => style.getPropertyPriority(p) === 'important');
}

function borderDecls(cs: CSSStyleDeclaration, mapper: ColorMapper): Decl[] {
  const colors: Array<string | null> = SIDES.map((side) => {
    const style = cs.getPropertyValue(`border-${side}-style`);
    if (style === 'none' || style === 'hidden') return null;
    if (!(parseFloat(cs.getPropertyValue(`border-${side}-width`)) > 0)) return null;
    const value = cs.getPropertyValue(`border-${side}-color`);
    const mapped = mapper.map(value, 'border');
    return mapped && mapped !== value ? mapped : null;
  });
  if (colors.every((c) => c !== null && c === colors[0])) {
    return [['border-color', colors[0] as string]];
  }
  const decls: Decl[] = [];
  colors.forEach((c, i) => {
    if (c) decls.push([`border-${SIDES[i]}-color`, c]);
  });
  return decls;
}

function longhands(prop: string) {
  return prop === 'border-color' ? SIDES.map((s) => `border-${s}-color`) : [prop];
}

/**
 * Darkens `doc` in place for display on `palette`. Emails that are already dark are left
 * alone. Safe to call repeatedly; each call first removes the previous darkening.
 */
export function darkenDocument(
  doc: Document,
  palette: Palette,
  options: DarkenOptions = {}
): DarkenResult {
  const win = doc.defaultView;
  const root = options.root || doc.body;
  if (!win || !root) return { status: 'darkened', elements: 0 };
  removeDarkening(doc);

  const colors = parsePalette(palette);
  const mapper = new ColorMapper(polesFor(colors.background, colors.text));
  const themeForegrounds = new Set([formatColor(colors.text), formatColor(colors.link)]);
  const maxElements = options.maxElements ?? DEFAULT_MAX_ELEMENTS;

  const page = pageColor(root, win);
  const rootRect = root.getBoundingClientRect();
  const byArea = rootRect.width * rootRect.height > 0;
  const regions: PaintedRegion[] = [
    { color: page, area: byArea ? rootRect.width * rootRect.height : 1 },
  ];
  const records: ElementRecord[] = [];

  const stack: Frame[] = [];
  const pushChildren = (frame: Frame) => {
    for (let c = frame.el.lastElementChild; c; c = c.previousElementSibling) {
      stack.push({ ...frame, el: c });
    }
  };
  pushChildren({ el: root, untouched: false, region: 0, regionColor: page });

  let count = 0;
  let overflow = false;

  const addRegion = (frame: Frame, el: Element, color: RGBA | null) => {
    let area = 1;
    if (byArea) {
      const r = el.getBoundingClientRect();
      area = r.width * r.height;
      regions[frame.region].area -= area;
    }
    regions.push({ color, area });
    return regions.length - 1;
  };

  while (stack.length) {
    const frame = stack.pop() as Frame;
    const { el } = frame;
    const tag = el.localName;
    if (SKIP_TAGS.has(tag) || MEDIA_TAGS.has(tag)) continue;
    if (++count > maxElements) {
      overflow = true;
      break;
    }
    const cs = win.getComputedStyle(el);
    if (cs.display === 'none' || cs.opacity === '0') continue;

    const next: Frame = { ...frame };
    const decls: Decl[] = [];
    const isSvg = el.namespaceURI === SVG_NS;

    if (!isSvg || tag === 'svg') {
      const bgImage = cs.backgroundImage;
      const bgValue = cs.backgroundColor;
      const bg = parseColor(bgValue);
      if (bgImage && bgImage !== 'none') {
        next.untouched = true;
        next.region = addRegion(frame, el, null);
      } else if (bg && bg.a > 0 && (!frame.untouched || bg.a >= 0.9)) {
        next.untouched = false;
        const mapped = mapper.map(bgValue, 'background');
        if (mapped && mapped !== bgValue) decls.push(['background-color', mapped]);
        if (bg.a >= 0.5) {
          next.regionColor = composite(bg, frame.regionColor);
          next.region = addRegion(frame, el, next.regionColor);
        }
      }
    }

    if (COLOR_TAGS.has(tag) || hasOwnText(el)) {
      const value = cs.color;
      if (next.untouched) {
        // Pin the sender's colour; otherwise it would inherit a mapped ancestor colour.
        decls.push(['color', value]);
      } else if (!themeForegrounds.has(value)) {
        const mapped = mapper.map(value, 'foreground');
        if (mapped && mapped !== value) decls.push(['color', mapped]);
      }
    }

    if (!next.untouched) {
      if (isSvg) {
        const fill = cs.getPropertyValue('fill');
        const stroke = cs.getPropertyValue('stroke');
        let fillRole: 'background' | 'foreground' = 'foreground';
        if (tag === 'rect') {
          const r = el.getBoundingClientRect();
          if (r.width >= 32 && r.height >= 32) fillRole = 'background';
        }
        const mappedFill = mapper.map(fill, fillRole);
        if (mappedFill && mappedFill !== fill) decls.push(['fill', mappedFill]);
        const mappedStroke = mapper.map(stroke, 'foreground');
        if (mappedStroke && mappedStroke !== stroke) decls.push(['stroke', mappedStroke]);
      } else {
        decls.push(...borderDecls(cs, mapper));
      }
    }

    if (decls.length) {
      const props = decls.flatMap(([p]) => longhands(p));
      records.push({ el, decls, inline: hasInlineImportant(el, props) });
    }
    pushChildren(next);
  }

  if (isAlreadyDark(regions)) return { status: 'already-dark', elements: count };

  const rules: string[] = [];
  const bgValue = formatColor(colors.background);
  const textValue = formatColor(colors.text);
  rules.push(
    `[${ROOT_ATTR}]${BOOST}{background-color:${bgValue}!important;color:${textValue}!important}`
  );

  if (overflow) {
    const border = mapper.map('rgb(204, 204, 204)', 'border');
    rules.push(
      `[${ROOT_ATTR}] *${BOOST}{background-color:transparent!important;color:${textValue}!important;border-color:${border}!important}`,
      `[${ROOT_ATTR}] a${BOOST},[${ROOT_ATTR}] a *${BOOST}{color:${formatColor(colors.link)}!important}`
    );
  } else {
    const ids = new Map<string, number>();
    for (const { el, decls, inline } of records) {
      if (inline) {
        el.setAttribute(SAVED_STYLE_ATTR, el.getAttribute('style') || '');
        const style = (el as HTMLElement).style;
        for (const [p, v] of decls) style.setProperty(p, v, 'important');
        continue;
      }
      const body = decls.map(([p, v]) => `${p}:${v}!important`).join(';');
      let id = ids.get(body);
      if (id === undefined) {
        id = ids.size;
        ids.set(body, id);
        rules.push(`[${ROOT_ATTR}] [${MARK_ATTR}="${id}"]${BOOST}{${body}}`);
      }
      el.setAttribute(MARK_ATTR, String(id));
    }
  }

  const brightness = options.imageBrightness ?? 1;
  if (brightness < 1) {
    rules.push(
      `[${ROOT_ATTR}] img${BOOST},[${ROOT_ATTR}] video${BOOST}{filter:brightness(${brightness})}`
    );
  }

  root.setAttribute(ROOT_ATTR, '');
  const sheet = doc.createElement('style');
  sheet.setAttribute(SHEET_ATTR, '');
  sheet.textContent = rules.join('\n');
  (doc.head || doc.documentElement).appendChild(sheet);

  if (options.restoreOnCopy !== false) {
    const onCopy = () => {
      sheet.media = 'not all';
      setTimeout(() => {
        sheet.media = '';
      }, 0);
    };
    doc.addEventListener('copy', onCopy, true);
    copyListeners.set(doc, onCopy);
  }

  return { status: overflow ? 'fallback' : 'darkened', elements: count };
}

function isDocument(target: Document | Element): target is Document {
  return target.nodeType === 9;
}

/**
 * Removes everything `darkenDocument` added under `target`, restoring inline styles it
 * overrode. Works on a detached clone as well as on the live document.
 */
export function removeDarkening(target: Document | Element) {
  const all = (selector: string): Element[] => {
    const found = Array.from(target.querySelectorAll(selector));
    if (!isDocument(target) && target.matches(selector)) found.unshift(target);
    return found;
  };
  for (const sheet of all(`style[${SHEET_ATTR}]`)) sheet.remove();
  for (const el of all(`[${SAVED_STYLE_ATTR}]`)) {
    const original = el.getAttribute(SAVED_STYLE_ATTR);
    if (original) el.setAttribute('style', original);
    else el.removeAttribute('style');
    el.removeAttribute(SAVED_STYLE_ATTR);
  }
  for (const el of all(`[${MARK_ATTR}]`)) el.removeAttribute(MARK_ATTR);
  for (const el of all(`[${ROOT_ATTR}]`)) el.removeAttribute(ROOT_ATTR);

  if (isDocument(target)) {
    const onCopy = copyListeners.get(target);
    if (onCopy) {
      target.removeEventListener('copy', onCopy, true);
      copyListeners.delete(target);
    }
  }
}

export function isDarkened(doc: Document) {
  return !!doc.querySelector(`style[${SHEET_ATTR}]`);
}
