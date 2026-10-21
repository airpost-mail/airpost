import * as ExtensionRegistry from '../../packages/extension-registry';
import { MessageViewExtension } from '../../packages/message-view-extension';
import { ComposerExtension } from '../../packages/composer-extension';
import { Message } from '../../data/models/message';
import { MailTrackerBlockerRules } from './mail-tracker-blocker-rules';

// Mailspring's original list, kept where the MailTrackerBlocker rules don't cover a pattern.
// Same format: case-insensitive regexes tested against each image URL.
export const AdditionalRules: { [vendor: string]: string[] } = {
  NGPVan: ['click\\.ngpvan\\.com/'],
  Yesware: ['app\\.yesware\\.com'],
  MailChimp: ['list-manage\\.com/track'],
  'Campaign Monitor': ['cmail1\\.com/t'],
  Mandrill: ['mandrillapp\\.com/track'],
  HubSpot: ['t\\.hsms06\\.com'],
  // Mailspring's opaque-token read receipts (/o/<token>.png); the /open/ form is in the list.
  Mailspring: ['getmailspring\\.com/o/'],
};

// The rules run on attacker-chosen URLs on the renderer thread, so their unbounded `.+` would
// let one crafted URL backtrack for seconds. Wildcards are capped and only the start of each
// URL is tested; tracker markers sit in the host and path, well inside it.
const MaxWildcard = 256;
const MaxTestedURL = 2048;

const AllRules = [...Object.values(MailTrackerBlockerRules), ...Object.values(AdditionalRules)]
  .flat()
  .map((rule) => rule.replace(/\.\+/g, `.{1,${MaxWildcard}}`));

function combine(rules: string[]) {
  return new RegExp(rules.map((rule) => `(?:${rule})`).join('|'), 'i');
}

const TrackerRegexp = combine(AllRules);

// Drafts keep only vendor rules: path-only ones ("/open.gif", "/1.gif", "/t/x/y") also match
// ordinary images, and a removed image is gone for good from the outgoing reply or forward.
const DraftTrackerRegexp = combine(AllRules.filter((rule) => !rule.startsWith('/')));

// Tags that can load an image: <img> (<image> parses as one), <source srcset> in a <picture>,
// and <input type=image>. Allows ">" inside quoted attribute values.
const ImageTagRegexp = /<(?:img|image|source|input)\b(?:[^>"']|"[^"]*"|'[^']*')*>/gi;
const AttributeRegexp = /([^\s"'<>/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+)))?/g;

function parseTag(tag: string) {
  const name = /^<([a-z]+)/i.exec(tag)[1].toLowerCase();
  const attrs: { [name: string]: string } = {};
  const inner = tag.replace(/^<[a-z]+/i, '').replace(/\/?>$/, '');
  let match: RegExpExecArray | null;
  AttributeRegexp.lastIndex = 0;
  while ((match = AttributeRegexp.exec(inner)) !== null) {
    const attr = match[1].toLowerCase();
    if (!(attr in attrs)) {
      attrs[attr] = match[2] ?? match[3] ?? match[4] ?? '';
    }
  }
  return { name, attrs };
}

// The URLs of a srcset value, per the HTML candidate grammar: a URL is a run of non-space
// characters (so data: URLs keep their commas), followed by descriptors up to the next comma.
export function srcsetURLs(srcset: string) {
  const urls: string[] = [];
  let i = 0;
  while (i < srcset.length) {
    while (i < srcset.length && /[\s,]/.test(srcset[i])) i++;
    let j = i;
    while (j < srcset.length && !/\s/.test(srcset[j])) j++;
    let url = srcset.slice(i, j);
    if (url.endsWith(',')) {
      url = url.replace(/,+$/, '');
    } else {
      while (j < srcset.length && srcset[j] !== ',') j++;
    }
    if (url) urls.push(url);
    i = j;
  }
  return urls;
}

function imageURLs(name: string, attrs: { [name: string]: string }) {
  if (name === 'input' && (attrs.type || '').trim().toLowerCase() !== 'image') return [];
  const urls = attrs.srcset ? srcsetURLs(attrs.srcset) : [];
  if (attrs.src !== undefined) urls.unshift(attrs.src);
  return urls;
}

function parseStyle(style: string) {
  const decls: { [prop: string]: string } = {};
  for (const decl of style.split(';')) {
    const idx = decl.indexOf(':');
    if (idx === -1) continue;
    const prop = decl.slice(0, idx).trim().toLowerCase();
    const value = decl
      .slice(idx + 1)
      .replace(/!important/i, '')
      .trim()
      .toLowerCase();
    if (prop) decls[prop] = value;
  }
  return decls;
}

// A size in pixels from a width/height attribute ("1", "1px") or CSS value ("1px", "0"):
// undefined when unset, Infinity when set in other units ("100%", "auto"). Negative sizes count
// as 0: Chromium ignores them and draws the image at its natural size, 1x1 for a pixel.
function pixelSize(value: string | undefined) {
  if (value === undefined) return undefined;
  const match = /^\s*(-?\d*\.?\d+)\s*(px)?\s*$/i.exec(value);
  return match ? Math.max(0, parseFloat(match[1])) : Infinity;
}

// Whether a URL can reach the network. Only inline and app schemes are local; anything else,
// including relative and malformed forms like "https:host" or "https:\\host" that Chromium
// resolves to a remote host, counts as remote.
export function isRemoteURL(url: string) {
  return !/^\s*(?:cid|data|tern-asset|mailspring|tern-file|file):/i.test(url) && url.trim() !== '';
}

// An image nobody is meant to see: hidden with display:none, visibility:hidden, opacity:0 or the
// hidden attribute, zero-sized, or at most 1x1 (max-width/max-height included). A missing
// dimension counts as 1 when the other is set, since a tracking pixel's natural size is 1x1.
function isHidden(attrs: { [name: string]: string }) {
  const style = parseStyle(attrs.style || '');
  if (
    style.display === 'none' ||
    style.visibility === 'hidden' ||
    parseFloat(style.opacity) === 0 ||
    'hidden' in attrs
  ) {
    return true;
  }
  const dimension = (prop: 'width' | 'height') => {
    const size = pixelSize(style[prop] ?? attrs[prop]);
    const max = pixelSize(style[`max-${prop}`]);
    return max === undefined || max === Infinity ? size : Math.min(size ?? max, max);
  };
  const width = dimension('width');
  const height = dimension('height');
  if (width === 0 || height === 0) return true;
  if (width === undefined && height === undefined) return false;
  return (width ?? 1) <= 1 && (height ?? 1) <= 1;
}

// Whether an image tag loads a known tracker or is a hidden remote image. Tags whose URLs are
// all local (cid:, data:) can't report anything and are left alone.
export function isTrackingImage(tag: string, trackers = TrackerRegexp) {
  const { name, attrs } = parseTag(tag);
  const remote = imageURLs(name, attrs).filter(isRemoteURL);
  if (remote.length === 0) return false;
  return remote.some((url) => trackers.test(url.slice(0, MaxTestedURL))) || isHidden(attrs);
}

export function rejectImagesInBody(body: string, callback: (tag: string) => boolean) {
  return body.replace(ImageTagRegexp, (tag) => (callback(tag) ? '' : tag));
}

// Runs on every message, from anyone: known tracker images and hidden remote images are removed
// before the body renders, so they don't load even when remote images are allowed.
export function removeTrackingPixels(message: Message, trackers = TrackerRegexp) {
  if (typeof message.body !== 'string') return;
  message.body = rejectImagesInBody(message.body, (tag) => isTrackingImage(tag, trackers));
}

export function removeTrackingPixelsFromDraft(draft: Message) {
  removeTrackingPixels(draft, DraftTrackerRegexp);
}

class TrackingPixelsMessageExtension extends MessageViewExtension {
  static formatMessageBody = ({ message }) => {
    removeTrackingPixels(message);
  };
}

class TrackingPixelsComposerExtension extends ComposerExtension {
  static prepareNewDraft = ({ draft }) => {
    removeTrackingPixelsFromDraft(draft);
  };
}

export function activate() {
  ExtensionRegistry.MessageView.register(TrackingPixelsMessageExtension);
  ExtensionRegistry.Composer.register(TrackingPixelsComposerExtension);
}

export function deactivate() {
  ExtensionRegistry.MessageView.unregister(TrackingPixelsMessageExtension);
  ExtensionRegistry.Composer.unregister(TrackingPixelsComposerExtension);
}
