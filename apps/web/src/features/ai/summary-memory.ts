import type { MessageJSON } from '@/engine';

/** One message this long is worth a summary on its own. */
const LONG_MESSAGE_CHARS = 4000;
const REMEMBERED = 100;

export interface Summary {
  text: string;
  model: string;
}

/**
 * Summaries this tab has seen, by thread and its newest message, so reopening a thread
 * shows one without a call. The engine caches them too (24 h); this only saves the trip.
 */
export const remembered = new Map<string, Summary>();

export function remember(key: string, s: Summary) {
  remembered.delete(key);
  remembered.set(key, s);
  while (remembered.size > REMEMBERED) remembered.delete(remembered.keys().next().value!);
}

/** For tests. */
export function forgetSummaries() {
  remembered.clear();
}

export function worthSummarizing(messages: MessageJSON[]): boolean {
  const sent = messages.filter((m) => !m.draft);
  if (sent.length >= 2) return true;
  return sent.length === 1 && (sent[0].body?.length ?? 0) >= LONG_MESSAGE_CHARS;
}
