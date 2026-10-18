export type ConnectionSecurity = 'SSL / TLS' | 'STARTTLS' | 'none';
export type MailProtocol = 'imap' | 'smtp';

// A cleartext connection to these never leaves the machine (Proton Bridge listens on
// 127.0.0.1), so `none` needs no confirmation there.
const LOOPBACK_HOSTS = ['127.0.0.1', 'localhost', '::1', '[::1]'];

export function isLoopbackHost(host: string | undefined) {
  return LOOPBACK_HOSTS.includes(`${host || ''}`.trim().toLowerCase());
}

/** The security a port implies when the user picks it, or null for a custom port. */
export function securityForPort(protocol: MailProtocol, port: number): ConnectionSecurity | null {
  if (protocol === 'imap') {
    if (port === 993) return 'SSL / TLS';
    if (port === 143) return 'STARTTLS';
  } else {
    if (port === 465) return 'SSL / TLS';
    if (port === 587 || port === 25) return 'STARTTLS';
  }
  return null;
}

/**
 * The security discovery stores for a server when its source says `value` (or nothing) on
 * `port`. Never `none`: a server without TLS fails the connection test, and the user can
 * opt into cleartext on the IMAP/SMTP settings page.
 */
export function discoveredSecurity(
  protocol: MailProtocol,
  value: string | undefined,
  port: number
): ConnectionSecurity {
  if (value === 'SSL / TLS' || value === 'STARTTLS') return value;
  return securityForPort(protocol, port) || 'STARTTLS';
}

/** The settings key that records the "send password unencrypted" confirmation. */
export function cleartextConsentKey(protocol: MailProtocol) {
  return `${protocol}_allow_cleartext`;
}

/** True when `protocol` is set to `none` for a remote host and the user hasn't confirmed it. */
export function needsCleartextConsent(settings: Record<string, any>, protocol: MailProtocol) {
  return (
    settings[`${protocol}_security`] === 'none' &&
    !isLoopbackHost(settings[`${protocol}_host`]) &&
    settings[cleartextConsentKey(protocol)] !== true
  );
}
