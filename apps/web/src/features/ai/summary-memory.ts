import type { EngineClient, MessageJSON } from '@/engine';

/** One message this long is worth a summary on its own. */
const LONG_MESSAGE_CHARS = 4000;
const REMEMBERED = 100;

export interface Summary {
  text: string;
  model: string;
  /** The destination note of the provider that wrote it. */
  where: string;
}

/**
 * Summaries this tab has seen, by thread and its newest message, so reopening a thread
 * shows one without a call. The engine caches them too (24 h); this only saves the trip,
 * so it is dropped whenever the engine's cache may have been: see watchSummaries.
 */
export const remembered = new Map<string, Summary>();

export function remember(key: string, s: Summary) {
  remembered.delete(key);
  remembered.set(key, s);
  while (remembered.size > REMEMBERED) remembered.delete(remembered.keys().next().value!);
}

export function forgetSummaries() {
  remembered.clear();
}

const PROVIDER_KEYS = ['core.ai.providers', 'core.ai.features', 'core.ai.accounts'];
const watched = new WeakSet<EngineClient>();

/**
 * Forgets the summaries on a reconnect (anything may have changed meanwhile), when an
 * account restarts (a reset drops the engine's cache and thread ids come back the same),
 * and when the providers, switches or opt-ins change.
 */
export function watchSummaries(client: EngineClient) {
  if (watched.has(client)) return;
  watched.add(client);
  let opened = client.state === 'open';
  client.onStateChange((s) => {
    if (s !== 'open') return;
    if (opened) forgetSummaries();
    opened = true;
  });
  client.onNotification((n) => {
    if (n.method === 'account.state' && n.params.state === 'starting') forgetSummaries();
    else if (
      n.method === 'settings.changed' &&
      Object.keys(n.params.values).some((k) => PROVIDER_KEYS.includes(k))
    )
      forgetSummaries();
  });
}

export function worthSummarizing(messages: MessageJSON[]): boolean {
  const sent = messages.filter((m) => !m.draft);
  if (sent.length >= 2) return true;
  return sent.length === 1 && (sent[0].body?.length ?? 0) >= LONG_MESSAGE_CHARS;
}
