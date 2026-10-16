/** Hosts where plain http is acceptable (same rule as the engine's Tern server URL check). */
const LOOPBACK = new Set(['localhost', '127.0.0.1', '[::1]']);

/**
 * The Tern server the login form starts with: the http(s) origin of VITE_ENGINE_URL when a
 * hosted engine is configured, otherwise the origin that served this page.
 */
export function defaultServerUrl(): string {
  const engineUrl = import.meta.env.VITE_ENGINE_URL;
  if (engineUrl) {
    try {
      const u = new URL(engineUrl);
      u.protocol = u.protocol === 'wss:' ? 'https:' : u.protocol === 'ws:' ? 'http:' : u.protocol;
      return u.origin;
    } catch {
      // fall through to the page origin
    }
  }
  return window.location.origin;
}

export type ServerUrlResult = { ok: true; url: string } | { ok: false; reason: string };

/** Accepts "tern.example.com" or a full URL; returns the base URL without a trailing slash. */
export function parseServerUrl(input: string): ServerUrlResult {
  const raw = input.trim();
  if (!raw) return { ok: false, reason: 'Enter the address of your Tern server.' };
  let u: URL;
  try {
    u = new URL(/^[a-z][a-z\d+.-]*:\/\//i.test(raw) ? raw : `https://${raw}`);
  } catch {
    return { ok: false, reason: 'That is not a valid address.' };
  }
  if (u.protocol !== 'https:' && u.protocol !== 'http:') {
    return { ok: false, reason: 'Use an https:// address.' };
  }
  if (u.protocol === 'http:' && !LOOPBACK.has(u.hostname)) {
    // Tokens travel in an Authorization header; over plain http they leak.
    return { ok: false, reason: 'Use https:// unless the server is on this machine.' };
  }
  if (u.username || u.password)
    return { ok: false, reason: 'Leave credentials out of the address.' };
  const path = u.pathname.replace(/\/+$/, '');
  return { ok: true, url: `${u.origin}${path}` };
}

/** Host for display: "tern.example.com", or "localhost:5173". */
export function serverLabel(url: string): string {
  try {
    const u = new URL(url);
    return `${u.host}${u.pathname === '/' ? '' : u.pathname}`;
  } catch {
    return url;
  }
}

/** Keeps `?redirect=` to paths on this origin so the login can't bounce users elsewhere. */
export function safeRedirect(value: unknown): string | undefined {
  if (typeof value !== 'string' || !value.startsWith('/') || value.startsWith('//')) {
    return undefined;
  }
  if (value.startsWith('/\\') || value.startsWith('/login')) return undefined;
  return value;
}
