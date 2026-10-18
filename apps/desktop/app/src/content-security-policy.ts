import { FilesScheme } from './files-scheme';

// The app's policy. main.js sends it as a response header and every static/*.html page
// carries the same string in its <meta> tag; spec/content-security-policy-spec.ts fails if
// they drift.
//
// connect-src lists the hosts the renderer fetches itself. Requests to hosts that depend on
// user data (autoconfig, one-click unsubscribe, saving a remote image) go through
// ExternalFetch in the main process instead.
const ConnectSources = [
  "'self'",
  'mailspring:',
  'https://www.googleapis.com', // Gmail token exchange and userinfo
  'https://oauth2.googleapis.com', // Google token revocation
  'https://login.microsoftonline.com', // Microsoft token exchange
  'https://graph.microsoft.com', // Microsoft profile
  'https://api.github.com', // github-contact-card
];

export const AppContentSecurityPolicy =
  [
    "default-src 'self' mailspring:",
    "script-src 'self'",
    "style-src * 'unsafe-inline' mailspring:",
    `img-src * data: mailspring: file: ${FilesScheme}:`,
    'font-src * data: mailspring:',
    `connect-src ${ConnectSources.join(' ')}`,
    // Message bodies render in src-less about:blank frames, which frame-src doesn't govern.
    "frame-src 'none'",
    "object-src 'none'",
    'media-src mailspring:',
    "manifest-src 'none'",
  ].join('; ') + ';';

function remoteSources(remoteImages: boolean) {
  return remoteImages ? ' https: http:' : '';
}

/**
 * The policy written first into every document that shows a message body (the reading pane
 * iframe, the clipped-message and show-original windows). Remote images and fonts load only
 * when `remoteImages` is set for that message. Message documents are about:blank children
 * of the app window, so the app policy above applies too and must allow at least this much.
 */
export function messageContentSecurityPolicy({ remoteImages }: { remoteImages: boolean }) {
  const remote = remoteSources(remoteImages);
  return [
    "default-src 'none'",
    "style-src 'unsafe-inline'",
    `img-src data: mailspring: ${FilesScheme}:${remote}`,
    `font-src data: mailspring:${remote}`,
    "form-action 'none'",
  ].join('; ');
}

/** The print window: the message policy plus its own scripts and stylesheets. */
export function printContentSecurityPolicy({ remoteImages }: { remoteImages: boolean }) {
  const remote = remoteSources(remoteImages);
  return [
    "default-src 'none'",
    "script-src 'self'",
    "style-src 'self' 'unsafe-inline' mailspring:",
    `img-src data: mailspring: ${FilesScheme}:${remote}`,
    `font-src data: mailspring:${remote}`,
    "form-action 'none'",
  ].join('; ');
}

/** Attachment quick preview (pdf.js, docx, markdown, sheets, code). */
export function quickPreviewContentSecurityPolicy({ remoteImages }: { remoteImages: boolean }) {
  const remote = remoteSources(remoteImages);
  return [
    "default-src 'self'",
    "script-src 'self' 'unsafe-inline'", // the inline script in quickpreview/renderer.html
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
