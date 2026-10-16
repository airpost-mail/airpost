// Provider presets, from the desktop's onboarding table (mailspring-provider-settings.json).
import type { SecurityMode } from '@/engine';

export type UserFormat = 'email' | 'email-without-domain';

export interface ServerPreset {
  host: string;
  port: number;
  security: SecurityMode;
}

export interface ProviderPreset {
  id: string;
  name: string;
  /** `AccountJSON.provider`. */
  provider: string;
  domains: string[];
  imap?: ServerPreset;
  smtp?: ServerPreset;
  userFormat: UserFormat;
  /** One line shown above the password field. */
  passwordHint?: string;
}

// TODO(engine): Gmail and Microsoft sign in with OAuth on the desktop. From the web it is
// deferred (docs/web-gateway-plan.md section 4): a gateway on any host can't be a registered
// redirect URI, so it waits for a fixed redirect page and DCP stage 12's `account.oauth.*`.
// Until then these presets use IMAP with an app password.
export const PRESETS: ProviderPreset[] = [
  {
    id: 'gmail',
    name: 'Gmail',
    provider: 'gmail',
    domains: ['gmail.com', 'googlemail.com'],
    imap: { host: 'imap.gmail.com', port: 993, security: 'SSL / TLS' },
    smtp: { host: 'smtp.gmail.com', port: 465, security: 'SSL / TLS' },
    userFormat: 'email',
    passwordHint: 'Use an app password from your Google account. Sign in with Google is coming.',
  },
  {
    id: 'office365',
    name: 'Microsoft 365',
    provider: 'office365',
    domains: ['outlook.com', 'hotmail.com', 'live.com', 'msn.com'],
    imap: { host: 'outlook.office365.com', port: 993, security: 'SSL / TLS' },
    smtp: { host: 'smtp.office365.com', port: 587, security: 'STARTTLS' },
    userFormat: 'email',
    passwordHint: 'Most Microsoft accounts need Microsoft sign-in, which is coming.',
  },
  {
    id: 'icloud',
    name: 'iCloud',
    provider: 'icloud',
    domains: ['icloud.com', 'me.com', 'mac.com'],
    imap: { host: 'imap.mail.me.com', port: 993, security: 'SSL / TLS' },
    smtp: { host: 'smtp.mail.me.com', port: 587, security: 'STARTTLS' },
    userFormat: 'email',
    passwordHint: 'Use an app-specific password from your Apple account.',
  },
  {
    id: 'fastmail',
    name: 'Fastmail',
    provider: 'fastmail',
    domains: ['fastmail.com', 'fastmail.fm'],
    imap: { host: 'imap.fastmail.com', port: 993, security: 'SSL / TLS' },
    smtp: { host: 'smtp.fastmail.com', port: 465, security: 'SSL / TLS' },
    userFormat: 'email',
    passwordHint: 'Use an app password from Fastmail’s settings.',
  },
  {
    id: 'yahoo',
    name: 'Yahoo',
    provider: 'yahoo',
    domains: ['yahoo.com', 'ymail.com'],
    imap: { host: 'imap.mail.yahoo.com', port: 993, security: 'SSL / TLS' },
    smtp: { host: 'smtp.mail.yahoo.com', port: 465, security: 'SSL / TLS' },
    userFormat: 'email',
    passwordHint: 'Use an app password from Yahoo’s account security page.',
  },
  {
    id: 'imap',
    name: 'Other IMAP',
    provider: 'imap',
    domains: [],
    userFormat: 'email',
  },
];

export const OTHER_PRESET = PRESETS.find((p) => p.id === 'imap')!;

export function presetById(id: string | undefined): ProviderPreset | undefined {
  return PRESETS.find((p) => p.id === id);
}

export function domainOf(email: string): string {
  const at = email.lastIndexOf('@');
  return at === -1
    ? ''
    : email
        .slice(at + 1)
        .trim()
        .toLowerCase();
}

/** The preset for an address's domain, or Other IMAP. */
export function presetForEmail(email: string): ProviderPreset {
  const d = domainOf(email);
  return PRESETS.find((p) => p.domains.includes(d)) ?? OTHER_PRESET;
}

// TODO(engine): the desktop also consults mailcore's provider table and MX records;
// `account.discover` (DCP stage 12) replaces this guess.
export function guessServers(email: string): { imap: ServerPreset; smtp: ServerPreset } {
  const d = domainOf(email);
  return {
    imap: { host: d ? `imap.${d}` : '', port: 993, security: 'SSL / TLS' },
    smtp: { host: d ? `smtp.${d}` : '', port: 587, security: 'STARTTLS' },
  };
}

export function usernameFor(email: string, format: UserFormat): string {
  const e = email.trim();
  return format === 'email-without-domain' ? e.split('@')[0] : e;
}
