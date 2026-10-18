import {
  ExtensionRegistry,
  MessageViewExtension,
  ComposerExtension,
  Message,
} from 'mailspring-exports';
import { MailTrackerBlockerRules } from './mail-tracker-blocker-rules';

// Mailspring's original list, kept where the MailTrackerBlocker rules don't cover a pattern.
// Same format: case-insensitive regexes tested against the whole <img> tag.
const AdditionalRules: { [vendor: string]: string[] } = {
  NGPVan: ['click\\.ngpvan\\.com/'],
  Yesware: ['app\\.yesware\\.com'],
  MailChimp: ['list-manage\\.com/track'],
  'Campaign Monitor': ['cmail1\\.com/t'],
  Mandrill: ['mandrillapp\\.com/track'],
  HubSpot: ['t\\.hsms06\\.com'],
  // Mailspring's opaque-token read receipts (/o/<token>.png); the /open/ form is in the list.
  Mailspring: ['getmailspring\\.com/o/'],
};

const TrackerRegexp = new RegExp(
  [...Object.values(MailTrackerBlockerRules), ...Object.values(AdditionalRules)]
    .flat()
    .map((rule) => `(?:${rule})`)
    .join('|'),
  'i'
);

// An <img> tag, allowing ">" inside quoted attribute values.
const ImgTagRegexp = /<img\b(?:[^>"']|"[^"]*"|'[^']*')*>/gi;
const AttributeRegexp = /([^\s"'<>/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+)))?/g;

function parseImgAttributes(tag: string) {
  const attrs: { [name: string]: string } = {};
  const inner = tag.replace(/^<img\b/i, '').replace(/\/?>$/, '');
  let match: RegExpExecArray | null;
  AttributeRegexp.lastIndex = 0;
  while ((match = AttributeRegexp.exec(inner)) !== null) {
    const name = match[1].toLowerCase();
    if (!(name in attrs)) {
      attrs[name] = match[2] ?? match[3] ?? match[4] ?? '';
    }
  }
  return attrs;
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
// undefined when unset, Infinity when set in other units ("100%", "auto").
function pixelSize(value: string | undefined) {
  if (value === undefined) return undefined;
  const match = /^\s*(\d+(?:\.\d+)?)\s*(px)?\s*$/i.exec(value);
  return match ? parseFloat(match[1]) : Infinity;
}

export function isRemoteURL(url: string) {
  return /^\s*(https?:)?\/\//i.test(url);
}

// A remote image nobody is meant to see: hidden with display:none, visibility:hidden or the
// hidden attribute, zero-sized, or 1x1. A missing dimension counts as 1 when the other is set,
// since a tracking pixel's natural size is 1x1.
function isHidden(attrs: { [name: string]: string }) {
  const style = parseStyle(attrs.style || '');
  if (style.display === 'none' || style.visibility === 'hidden' || 'hidden' in attrs) {
    return true;
  }
  const width = pixelSize(style.width ?? attrs.width);
  const height = pixelSize(style.height ?? attrs.height);
  if (width === 0 || height === 0) return true;
  if (width === undefined && height === undefined) return false;
  return (width ?? 1) <= 1 && (height ?? 1) <= 1;
}

// Whether an <img> tag is a known tracker or a hidden remote image. Images that aren't remote
// (cid:, data:) can't report anything and are left alone.
export function isTrackingImage(tag: string) {
  const attrs = parseImgAttributes(tag);
  if (!isRemoteURL(attrs.src || '')) return false;
  return TrackerRegexp.test(tag) || isHidden(attrs);
}

export function rejectImagesInBody(body: string, callback: (tag: string) => boolean) {
  return body.replace(ImgTagRegexp, (tag) => (callback(tag) ? '' : tag));
}

// Runs on every message, from anyone: known tracker images and hidden remote images are removed
// before the body renders, so they never load even when remote images are allowed.
export function removeTrackingPixels(message: Message) {
  if (typeof message.body !== 'string') return;
  message.body = rejectImagesInBody(message.body, isTrackingImage);
}

class TrackingPixelsMessageExtension extends MessageViewExtension {
  static formatMessageBody = ({ message }) => {
    removeTrackingPixels(message);
  };
}

class TrackingPixelsComposerExtension extends ComposerExtension {
  static prepareNewDraft = ({ draft }) => {
    removeTrackingPixels(draft);
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
