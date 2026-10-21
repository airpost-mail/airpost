import { FilesScheme } from './files-scheme';
import { MessageFrameOrigin, MessageFrameScriptURL } from './frame/protocol';

// The app's policy. main.js sends it as a response header and every static/*.html page
// carries the same string in its <meta> tag; tests/mail/content-security-policy.test.ts fails if
// they drift.
//
// connect-src lists the hosts the renderer fetches itself. Requests to hosts that depend on
// user data (autoconfig, one-click unsubscribe, saving a remote image) go through
// ExternalFetch in the main process instead.
// Package assets are tern-asset: URLs; mailspring: is the v1 plugin API name of that scheme.
const ConnectSources = [
  "'self'",
  'tern-asset:',
  'mailspring:',
  'https://www.googleapis.com', // Gmail token exchange and userinfo
  'https://oauth2.googleapis.com', // Google token revocation
  'https://login.microsoftonline.com', // Microsoft token exchange
  'https://graph.microsoft.com', // Microsoft profile
];

export const AppContentSecurityPolicy =
  [
    "default-src 'self' tern-asset: mailspring:",
    "script-src 'self'",
    "style-src * 'unsafe-inline' tern-asset: mailspring:",
    `img-src * data: tern-asset: mailspring: file: ${FilesScheme}:`,
    'font-src * data: tern-asset: mailspring:',
    `connect-src ${ConnectSources.join(' ')}`,
    // Message bodies render in the sandboxed message frame (src/mail/frame). Other
    // frames are src-less about:blank ones, which frame-src doesn't govern.
    `frame-src ${MessageFrameOrigin}`,
    "object-src 'none'",
    'media-src tern-asset: mailspring:',
    "manifest-src 'none'",
  ].join('; ') + ';';

function remoteSources(remoteImages: boolean) {
  return remoteImages ? ' https: http:' : '';
}

/**
 * The policy of every document that shows a message body (the reading pane's message frame,
 * the clipped-message and show-original windows). Remote images and fonts load only when
 * `remoteImages` is set for that message. The clipped and show-original documents are
 * about:blank children of an app window, so the app policy above applies to them too and
 * must allow at least this much.
 */
export function messageContentSecurityPolicy({ remoteImages }: { remoteImages: boolean }) {
  const remote = remoteSources(remoteImages);
  return [
    "default-src 'none'",
    "style-src 'unsafe-inline'",
    `img-src data: tern-asset: mailspring: ${FilesScheme}:${remote}`,
    `font-src data: tern-asset: mailspring:${remote}`,
    "form-action 'none'",
  ].join('; ');
}

/**
 * The message frame's policy: the message policy plus the frame's own script, and nothing
 * that could change where the frame's relative URLs point.
 */
export function messageFrameContentSecurityPolicy({ remoteImages }: { remoteImages: boolean }) {
  return [
    messageContentSecurityPolicy({ remoteImages }),
    `script-src ${MessageFrameScriptURL}`,
    "base-uri 'none'",
  ].join('; ');
}

/** The print window: the message policy plus its own scripts and stylesheets. */
export function printContentSecurityPolicy({ remoteImages }: { remoteImages: boolean }) {
  const remote = remoteSources(remoteImages);
  return [
    "default-src 'none'",
    "script-src 'self'",
    "style-src 'self' 'unsafe-inline' tern-asset: mailspring:",
    `img-src data: tern-asset: mailspring: ${FilesScheme}:${remote}`,
    `font-src data: tern-asset: mailspring:${remote}`,
    "form-action 'none'",
  ].join('; ');
}

/** Attachment quick preview (pdf.js, docx, markdown, sheets, code). */
export function quickPreviewContentSecurityPolicy({ remoteImages }: { remoteImages: boolean }) {
  const remote = remoteSources(remoteImages);
  return [
    "default-src 'self'",
    "script-src 'self' 'unsafe-inline'", // the inline script in mail/quick-preview/renderer.html
    "style-src 'self' 'unsafe-inline'",
    `img-src 'self' data:${remote}`,
    `font-src 'self' data:${remote}`,
    "object-src 'none'",
    "frame-src 'none'",
    "base-uri 'self'",
    "form-action 'none'",
  ].join('; ');
}

export function contentSecurityPolicyMetaTag(policy: string) {
  return `<meta http-equiv="Content-Security-Policy" content="${policy}">`;
}
