import { contactDisplayName, type AccountJSON, type ThreadJSON } from '@/engine';

/** The first participant who is not the account owner (falls back to the first). */
export function threadSender(thread: ThreadJSON, account?: AccountJSON): string {
  const own = new Set(
    [account?.emailAddress, ...(account?.aliases ?? [])]
      .filter((e): e is string => !!e)
      .map((e) => e.toLowerCase()),
  );
  const p =
    thread.participants.find((c) => !own.has(c.email.toLowerCase())) ?? thread.participants[0];
  return p ? contactDisplayName(p) : '';
}
