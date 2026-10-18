import { contactDisplayName, type ContactRef, type ThreadJSON } from '@/engine';

// TODO(engine): Thread JSON carries no message count and no per-message senders, so the
// row guesses the count from category refs and names participants rather than senders.
// `query` could read each page's messages, but that is a full Message read per list page
// for two fields; a count and senders on Thread JSON is the fix, and no DCP stage has it.
/** Messages in the thread, from its category refs. */
export function messageCount(t: ThreadJSON): number {
  return Math.max(1, ...[...t.folders, ...t.labels].map((r) => r._refs));
}

function firstName(c: ContactRef) {
  const name = contactDisplayName(c);
  return c.name?.trim() ? name.split(/\s+/)[0] : name.split('@')[0];
}

/** "Jonas", "Jonas, Priya", "Jonas, Priya +2", or "me" when the user is the only party. */
export function senderLabel(t: ThreadJSON, own: ReadonlySet<string>): string {
  const others = t.participants.filter((p) => !own.has(p.email.toLowerCase()));
  if (others.length === 0) return 'me';
  if (others.length === 1) return contactDisplayName(others[0]);
  const shown = others.slice(0, 2).map(firstName).join(', ');
  return others.length > 2 ? `${shown} +${others.length - 2}` : shown;
}
