// The addresses a draft can be sent from: each account's own address plus its aliases.
// Ported from the desktop's Account.defaultMe / meUsingAlias and DraftFactory._fromContactForReply.
import type { AccountJSON, ContactRef, MessageJSON } from '@/engine';
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

export function defaultIdentity(account: AccountJSON): Identity {
  const alias = account.defaultAlias ? aliasContact(account, account.defaultAlias) : null;
  return {
    accountId: account.id,
    contact: alias ?? { name: account.name, email: account.emailAddress },
  };
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

/** For a reply: the alias the original was addressed to, else the account default. */
export function replyIdentity(account: AccountJSON, message: MessageJSON): Identity {
  const fallback = defaultIdentity(account);
  const recipients = new Set([...message.to, ...message.cc].map((c) => emailKey(c.email)));
  const match = identitiesFor(account).find(
    (i) =>
      recipients.has(emailKey(i.contact.email)) &&
      emailKey(i.contact.email) !== emailKey(fallback.contact.email),
  );
  return match ?? fallback;
}
