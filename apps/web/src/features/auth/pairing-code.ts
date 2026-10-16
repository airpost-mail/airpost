// Pairing codes from `tern-gateway pair`: 12 base32 characters (RFC 4648 alphabet, 60 bits),
// printed in groups of four (docs/web-gateway-plan.md 2.3).

const LENGTH = 12;
const ALPHABET = /^[A-Z2-7]+$/;

/** The code as sent to the gateway (12 upper-case characters), or null if it isn't one. */
export function normalizePairingCode(input: string): string | null {
  const code = input.replace(/[\s-]+/g, '').toUpperCase();
  return code.length === LENGTH && ALPHABET.test(code) ? code : null;
}

/** `ABCD-EFGH-JKLM` for display. */
export function formatPairingCode(code: string): string {
  return code.match(/.{1,4}/g)?.join('-') ?? code;
}

/** The code in a `/pair#<code>` link, if the fragment holds one. */
export function codeFromHash(hash: string): string | null {
  return normalizePairingCode(decodeURIComponent(hash.replace(/^#/, '')));
}

/** "Firefox on macOS", from the User-Agent; the gateway shows it in `devices list`. */
export function defaultDeviceName(ua: string = navigator.userAgent): string {
  const browser = /Edg\//.test(ua)
    ? 'Edge'
    : /Firefox\//.test(ua)
      ? 'Firefox'
      : /Chrome\//.test(ua)
        ? 'Chrome'
        : /Safari\//.test(ua)
          ? 'Safari'
          : 'Browser';
  const os = /iPhone|iPad/.test(ua)
    ? 'iOS'
    : /Android/.test(ua)
      ? 'Android'
      : /Mac OS X|Macintosh/.test(ua)
        ? 'macOS'
        : /Windows/.test(ua)
          ? 'Windows'
          : /Linux/.test(ua)
            ? 'Linux'
            : null;
  return os ? `${browser} on ${os}` : browser;
}
