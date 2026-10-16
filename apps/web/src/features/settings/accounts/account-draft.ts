import type { AccountJSON, AccountSettingsJSON, SecurityMode } from '@/engine';
import { presetForEmail, guessServers, usernameFor, type ProviderPreset } from './presets';

export interface AccountForm {
  name: string;
  email: string;
  password: string;
  imapHost: string;
  imapPort: string;
  imapSecurity: SecurityMode;
  imapUsername: string;
  smtpHost: string;
  smtpPort: string;
  smtpSecurity: SecurityMode;
  smtpUsername: string;
  /** Empty means the IMAP password. */
  smtpPassword: string;
  allowInsecureSsl: boolean;
}

/** Fills server fields from the preset (or a guess from the domain for Other IMAP). */
export function formForPreset(
  preset: ProviderPreset,
  prev: Partial<AccountForm> = {},
): AccountForm {
  const email = prev.email ?? '';
  const servers =
    preset.imap && preset.smtp ? { imap: preset.imap, smtp: preset.smtp } : guessServers(email);
  const user = usernameFor(email, preset.userFormat);
  return {
    name: prev.name ?? '',
    email,
    password: prev.password ?? '',
    imapHost: servers.imap.host,
    imapPort: String(servers.imap.port),
    imapSecurity: servers.imap.security,
    imapUsername: user,
    smtpHost: servers.smtp.host,
    smtpPort: String(servers.smtp.port),
    smtpSecurity: servers.smtp.security,
    smtpUsername: user,
    smtpPassword: prev.smtpPassword ?? '',
    allowInsecureSsl: prev.allowInsecureSsl ?? false,
  };
}

export type FormErrors = Partial<Record<keyof AccountForm, string>>;

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function validPort(p: string) {
  const n = Number(p);
  return Number.isInteger(n) && n > 0 && n < 65536;
}

export function validateForm(f: AccountForm): FormErrors {
  const e: FormErrors = {};
  if (!EMAIL_RE.test(f.email.trim())) e.email = 'Enter an email address.';
  if (!f.password) e.password = 'Enter the password.';
  if (!f.imapHost.trim()) e.imapHost = 'Required.';
  if (!validPort(f.imapPort)) e.imapPort = 'Port 1–65535.';
  if (!f.smtpHost.trim()) e.smtpHost = 'Required.';
  if (!validPort(f.smtpPort)) e.smtpPort = 'Port 1–65535.';
  return e;
}

/**
 * Same id as the desktop's `idForAccount`: the first 8 hex chars of
 * sha256(email + JSON of the fields that could change the mailbox contents).
 */
export async function accountIdFor(email: string, s: AccountSettingsJSON): Promise<string> {
  const critical = {
    imap_username: s.imap_username,
    imap_host: s.imap_host,
    smtp_username: s.smtp_username,
    smtp_host: s.smtp_host,
  };
  const bytes = new TextEncoder().encode(`${email}${JSON.stringify(critical)}`);
  const digest = await crypto.subtle.digest('SHA-256', bytes);
  return [...new Uint8Array(digest)]
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('')
    .slice(0, 8);
}

/** Account dot colours, picked in turn for new accounts. */
export const ACCOUNT_COLORS = ['#3b6fe0', '#d97a2b', '#2f9e74', '#b84f9c', '#c9a227', '#5b6b82'];

export async function buildAccount(
  f: AccountForm,
  opts: { provider?: string; color?: string } = {},
): Promise<AccountJSON> {
  const email = f.email.trim();
  const settings: AccountSettingsJSON = {
    imap_host: f.imapHost.trim(),
    imap_port: Number(f.imapPort),
    imap_username: f.imapUsername.trim() || email,
    imap_password: f.password,
    imap_security: f.imapSecurity,
    imap_allow_insecure_ssl: f.allowInsecureSsl,
    smtp_host: f.smtpHost.trim(),
    smtp_port: Number(f.smtpPort),
    smtp_username: f.smtpUsername.trim() || email,
    smtp_password: f.smtpPassword || f.password,
    smtp_security: f.smtpSecurity,
    smtp_allow_insecure_ssl: f.allowInsecureSsl,
  };
  // TODO(engine): the engine assigns the id on `account.add` from DCP stages 11/12; this
  // mirrors the desktop.
  const id = await accountIdFor(email, settings);
  return {
    id,
    aid: id,
    v: 0,
    __cls: 'Account',
    name: f.name.trim() || email,
    provider: opts.provider ?? presetForEmail(email).provider,
    emailAddress: email,
    label: email,
    settings,
    autoaddress: { value: '', type: 'bcc' },
    aliases: [],
    defaultAlias: null,
    syncState: 'ok',
    syncError: null,
    color: opts.color ?? ACCOUNT_COLORS[0],
  };
}
