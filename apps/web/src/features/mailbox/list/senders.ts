import { contactDisplayName, type ContactRef, type ThreadJSON } from '@/engine';

// A list row reads the engine's `summary` (live thread lists ask for it); a thread from
// elsewhere (a search result) falls back to its category refs and participants.

/** Visible messages in the thread, drafts included. */
export function messageCount(t: ThreadJSON): number {
  if (t.summary) return t.summary.messageCount;
  return Math.max(1, ...[...t.folders, ...t.labels].map((r) => r._refs));
}

/** The last visible message's snippet. */
export function threadSnippet(t: ThreadJSON): string | null {
  if (t.summary) return t.summary.snippet || null;
  return t.snippet || null;
}

export function threadHasAttachment(t: ThreadJSON): boolean {
  return t.summary ? t.summary.hasAttachment : t.attachmentCount > 0;
}

function firstName(c: ContactRef) {
  const name = contactDisplayName(c);
  return c.name?.trim() ? name.split(/\s+/)[0] : name.split('@')[0];
}

/** The people a row names, the user left out: the summary's senders, else the participants. */
function others(t: ThreadJSON, own: ReadonlySet<string>): ContactRef[] {
  if (!t.summary) return t.participants.filter((p) => !own.has(p.email.toLowerCase()));
  const seen = new Set<string>();
  return t.summary.senders.filter((s) => {
    const email = s.email.toLowerCase();
    if (s.me || own.has(email) || seen.has(email)) return false;
    seen.add(email);
    return true;
  });
}

/** "Jonas", "Jonas, Priya", "Jonas, Priya +2", or "me" when the user is the only party. */
export function senderLabel(t: ThreadJSON, own: ReadonlySet<string>): string {
  const people = others(t, own);
  if (people.length === 0) return 'me';
  if (people.length === 1) return contactDisplayName(people[0]);
  const shown = people.slice(0, 2).map(firstName).join(', ');
  return people.length > 2 ? `${shown} +${people.length - 2}` : shown;
}
