// The addresses a draft can be sent from: each account's own address plus its aliases.
// Ported from the desktop's Account.meUsingAlias.
import type { AccountJSON, ContactRef } from '@/engine';
import { emailKey, parseAddress } from './recipients';

export interface Identity {
  accountId: string;
  contact: ContactRef;
}

export function identityKey(i: Identity): string {
  return `${i.accountId}|${emailKey(i.contact.email)}|${i.contact.name ?? ''}`;
}

function aliasContact(account: AccountJSON, alias: string): ContactRef | null {
  const c = parseAddress(alias);
  if (!c) return null;
  return { email: c.email, name: c.name ?? account.name };
}

export function identitiesFor(account: AccountJSON): Identity[] {
  const own: Identity = {
    accountId: account.id,
    contact: { name: account.name, email: account.emailAddress },
  };
  const aliases = account.aliases
    .map((a) => aliasContact(account, a))
    .filter((c): c is ContactRef => !!c)
    .map((contact) => ({ accountId: account.id, contact }));
  const seen = new Set<string>();
  return [own, ...aliases].filter((i) => {
    const k = identityKey(i);
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
}

export function allIdentities(accounts: AccountJSON[]): Identity[] {
  return accounts.flatMap(identitiesFor);
}

export function ownEmails(accounts: AccountJSON[]): Set<string> {
  return new Set(allIdentities(accounts).map((i) => emailKey(i.contact.email)));
}
