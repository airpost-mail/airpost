// SPDX-License-Identifier: GPL-3.0-only
//
// What an email says about its own colours: whether it supports dark mode and what colour it
// gives the page, read from markup and CSSOM rather than computed styles, because hosts
// usually force the frame's html/body transparent.

import { RGBA, parseColor } from './color';

const SCHEME_ATTR = 'data-email-dark-scheme';

export type ColorScheme = 'light' | 'dark';

type RuleVisitor = (rule: CSSStyleRule) => void;

// Visits style rules in document order, descending into @media blocks that currently match
// (so `prefers-color-scheme` blocks count only under the matching scheme) and into @supports
// and @layer blocks.
function eachActiveStyleRule(doc: Document, visit: RuleVisitor) {
  const win = doc.defaultView;
  const walk = (rules: CSSRuleList) => {
    for (const rule of Array.from(rules)) {
      const media = (rule as CSSMediaRule).media;
      if (media) {
        if (win && win.matchMedia(media.mediaText).matches) walk((rule as CSSMediaRule).cssRules);
      } else if ((rule as CSSStyleRule).selectorText !== undefined) {
        visit(rule as CSSStyleRule);
      } else if ((rule as CSSGroupingRule).cssRules) {
        walk((rule as CSSGroupingRule).cssRules);
      }
    }
  };
  for (const sheet of Array.from(doc.styleSheets)) {
    const owner = sheet.ownerNode as Element | null;
    if (owner && owner.hasAttribute && owner.hasAttribute(SCHEME_ATTR)) continue;
    try {
      walk(sheet.cssRules);
    } catch {
      // Cross-origin sheets can't be read; emails only have inline <style>.
    }
  }
}

function hasDarkMediaRule(rules: CSSRuleList): boolean {
  for (const rule of Array.from(rules)) {
    const media = (rule as CSSMediaRule).media;
    if (media && /prefers-color-scheme\s*:\s*dark/i.test(media.mediaText)) return true;
    const children = (rule as CSSGroupingRule).cssRules;
    if (children && hasDarkMediaRule(children)) return true;
  }
  return false;
}

/**
 * True when the email says it has a dark variant: a `color-scheme` meta tag or CSS property
 * that lists `dark`, or `@media (prefers-color-scheme: dark)` rules.
 */
export function declaresDarkSupport(doc: Document): boolean {
  for (const meta of Array.from(doc.querySelectorAll('meta[name]'))) {
    const name = (meta.getAttribute('name') || '').toLowerCase();
    if (name !== 'color-scheme' && name !== 'supported-color-schemes') continue;
    if (/\bdark\b/i.test(meta.getAttribute('content') || '')) return true;
  }
  for (const sheet of Array.from(doc.styleSheets)) {
    const owner = sheet.ownerNode as Element | null;
    if (owner && owner.hasAttribute && owner.hasAttribute(SCHEME_ATTR)) continue;
    let rules: CSSRuleList;
    try {
      rules = sheet.cssRules;
    } catch {
      continue;
    }
    if (hasDarkMediaRule(rules)) return true;
  }
  let declared = false;
  eachActiveStyleRule(doc, (rule) => {
    if (/\bdark\b/i.test(rule.style.getPropertyValue('color-scheme'))) declared = true;
  });
  return declared;
}

function resolveColor(doc: Document, value: string): RGBA | null {
  if (!value) return null;
  const parsed = parseColor(value);
  if (parsed) return parsed;
  const win = doc.defaultView;
  const host = doc.body || doc.documentElement;
  if (!win || !host) return null;
  const probe = doc.createElement('span');
  probe.style.color = value;
  if (!probe.style.color) return null;
  host.appendChild(probe);
  const resolved = parseColor(win.getComputedStyle(probe).color);
  probe.remove();
  return resolved;
}

function declaredBackground(doc: Document, el: Element | null): RGBA | null {
  if (!el) return null;
  const inline = (el as HTMLElement).style && (el as HTMLElement).style.backgroundColor;
  let found = inline ? resolveColor(doc, inline) : null;
  if (found && found.a > 0) return found;

  found = null;
  eachActiveStyleRule(doc, (rule) => {
    const value = rule.style.getPropertyValue('background-color');
    if (!value) return;
    let matches = false;
    try {
      matches = el.matches(rule.selectorText);
    } catch {
      return;
    }
    if (!matches) return;
    const color = resolveColor(doc, value);
    if (color && color.a > 0) found = color;
  });
  if (found) return found;

  const attr = el.getAttribute('bgcolor');
  const color = attr ? resolveColor(doc, attr) : null;
  return color && color.a > 0 ? color : null;
}

/**
 * The page colour the email asks for on <body> or <html>, by inline style, <style> rule or
 * `bgcolor`, whether or not the host lets it paint. Null when the email doesn't set one.
 */
export function declaredPageColor(doc: Document): RGBA | null {
  return declaredBackground(doc, doc.body) || declaredBackground(doc, doc.documentElement);
}

/**
 * Makes `prefers-color-scheme` inside `frame` follow `scheme` rather than the OS. Chromium
 * takes an iframe's preferred scheme from the iframe element's `color-scheme`, and paints an
 * opaque canvas behind a frame whose document root uses a different scheme, so both are set.
 * Call it after writing the document; it needs the document's root.
 */
export function applyColorScheme(frame: HTMLIFrameElement, scheme: ColorScheme) {
  frame.style.colorScheme = scheme;
  if (frame.contentDocument) applyDocumentColorScheme(frame.contentDocument, scheme);
}

/**
 * The document half of `applyColorScheme`, for code running inside a cross-origin frame;
 * the embedder sets the iframe element's `color-scheme` itself.
 */
export function applyDocumentColorScheme(doc: Document, scheme: ColorScheme) {
  if (!doc.documentElement) return;
  let style = doc.querySelector(`style[${SCHEME_ATTR}]`);
  if (!style) {
    style = doc.createElement('style');
    style.setAttribute(SCHEME_ATTR, '');
    (doc.head || doc.documentElement).appendChild(style);
  }
  style.textContent = `:root{color-scheme:${scheme}!important}`;
}

export function removeColorScheme(target: Document | Element) {
  for (const el of Array.from(target.querySelectorAll(`style[${SCHEME_ATTR}]`))) el.remove();
}
