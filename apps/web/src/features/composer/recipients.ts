// Address parsing for the recipient fields. Accepts what people paste from other clients:
// `Name <a@b.c>`, `"Last, First" <a@b.c>`, `<a@b.c>`, bare addresses and `mailto:` links,
// separated by commas, semicolons or newlines.
import type { ContactRef } from '@/engine';

const EMAIL_RE = /^[^\s@<>()",;:]+@[^\s@<>()",;:]+\.[^\s@<>()",;:]{2,}$/;

export function isValidEmail(email: string): boolean {
  return EMAIL_RE.test(email);
}

/** Splits on `,` `;` and newlines that are outside quotes and angle brackets. */
export function splitAddressList(input: string): string[] {
  const out: string[] = [];
  let current = '';
  let quoted = false;
  let angle = 0;
  for (const ch of input) {
    if (ch === '"') quoted = !quoted;
    else if (!quoted && ch === '<') angle++;
    else if (!quoted && ch === '>') angle = Math.max(0, angle - 1);
    if (!quoted && angle === 0 && (ch === ',' || ch === ';' || ch === '\n' || ch === '\r')) {
      out.push(current);
      current = '';
      continue;
    }
    current += ch;
  }
  out.push(current);
  return out.map((s) => s.trim()).filter(Boolean);
}

function stripMailto(s: string): string {
  return s.replace(/^mailto:/i, '').replace(/\?.*$/, '');
}

/** Parses one address. Returns null when there is no valid email in it. */
export function parseAddress(token: string): ContactRef | null {
  const s = token.trim();
  const angled = /^(.*?)<([^<>]+)>\s*$/.exec(s);
  if (angled) {
    const email = stripMailto(angled[2].trim());
    if (!isValidEmail(email)) return null;
    const name = angled[1]
      .trim()
      .replace(/^"(.*)"$/, '$1')
      .replace(/\\"/g, '"')
      .trim();
    return name && name !== email ? { name, email } : { email };
  }
  const email = stripMailto(s.replace(/^"(.*)"$/, '$1'));
  return isValidEmail(email) ? { email } : null;
}

export interface ParsedRecipients {
  contacts: ContactRef[];
  /** Tokens that are not addresses, in input order. */
  invalid: string[];
}

export function parseRecipients(input: string): ParsedRecipients {
  const contacts: ContactRef[] = [];
  const invalid: string[] = [];
  for (const token of splitAddressList(input)) {
    const c = parseAddress(token);
    if (c) contacts.push(c);
    else invalid.push(token);
  }
  return { contacts: dedupeContacts(contacts), invalid };
}

export function emailKey(email: string): string {
  return email.trim().toLowerCase();
}

/** Keeps the first occurrence of each address; later ones only fill a missing name. */
export function dedupeContacts(list: ContactRef[], exclude: Iterable<string> = []): ContactRef[] {
  const skip = new Set([...exclude].map(emailKey));
  const seen = new Map<string, ContactRef>();
  for (const c of list) {
    const key = emailKey(c.email);
    if (skip.has(key)) continue;
    const prev = seen.get(key);
    if (!prev) seen.set(key, { ...c });
    else if (!prev.name && c.name) prev.name = c.name;
  }
  return [...seen.values()];
}

/** `Name <email>`, quoting the name when it contains separators. */
export function formatContact(c: ContactRef): string {
  const name = c.name?.trim();
  if (!name || name === c.email) return c.email;
  const safe = /[,;<>"@]/.test(name) ? `"${name.replace(/"/g, '\\"')}"` : name;
  return `${safe} <${c.email}>`;
}

/** True when the text typed so far ends with a separator outside quotes and brackets. */
export function endsWithSeparator(text: string): boolean {
  let quoted = false;
  let angle = 0;
  let last = false;
  for (const ch of text) {
    if (ch === '"') quoted = !quoted;
    else if (!quoted && ch === '<') angle++;
    else if (!quoted && ch === '>') angle = Math.max(0, angle - 1);
    if (/\s/.test(ch)) continue;
    last = !quoted && angle === 0 && (ch === ',' || ch === ';');
  }
  return last;
}
