// The reader's Unsubscribe button: the engine's `messages.unsubscribe` acts on the
// List-Unsubscribe header (one-click POST, or the mailto message sent), and hands web links
// back for the browser to open. Links that appear only in the body aren't looked for, as on
// the desktop.
import { useState } from 'react';
import { toast } from 'sonner';
import {
  EngineFeature,
  ensureConnected,
  RpcError,
  RpcErrorCode,
  useEngine,
  useEngineFeatures,
  type MessageJSON,
  type MessagesUnsubscribeResult,
} from '@/engine';

/** Whether the header names a URI the engine can act on (RFC 2369: `<…>`, http(s) or mailto). */
export function hasUnsubscribeHeader(m: Pick<MessageJSON, 'hListUnsub'>): boolean {
  return [...(m.hListUnsub ?? '').matchAll(/<([^>]*)>/g)].some((x) =>
    /^\s*(https?:\/\/|mailto:)/i.test(x[1]!),
  );
}

function openLink(url: string) {
  if (/^https?:\/\//i.test(url)) window.open(url, '_blank', 'noopener,noreferrer');
}

const host = (url: string) => {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
};

/** Tells the user what happened. A web link opens only from the toast's button, a click. */
export function reportUnsubscribe(r: MessagesUnsubscribeResult) {
  const url = r.url ?? '';
  if (!r.ok) {
    toast.error(`Couldn't unsubscribe: ${r.error ?? 'the list refused'}`, {
      // A failed one-click link is usually also a page that unsubscribes.
      action:
        r.method === 'one-click' && url
          ? { label: 'Open link', onClick: () => openLink(url) }
          : undefined,
    });
    return;
  }
  switch (r.method) {
    case 'one-click':
      toast('Unsubscribed');
      break;
    case 'mailto':
      toast(`Sending the unsubscribe email to ${url.replace(/^mailto:/i, '').split('?')[0]}`);
      break;
    case 'web':
      toast(`This list unsubscribes on ${host(url)}`, {
        action: { label: 'Open', onClick: () => openLink(url) },
        duration: 15_000,
      });
      break;
  }
}

export function useUnsubscribe(m: MessageJSON) {
  const engine = useEngine();
  const features = useEngineFeatures();
  const [busy, setBusy] = useState(false);
  const available =
    !m.draft && !!features?.has(EngineFeature.Unsubscribe) && hasUnsubscribeHeader(m);

  const run = async () => {
    setBusy(true);
    try {
      await ensureConnected(engine);
      reportUnsubscribe(
        await engine.call('messages.unsubscribe', { accountId: m.aid, messageId: m.id }),
      );
    } catch (err) {
      toast.error(
        err instanceof RpcError && err.code === RpcErrorCode.NotFound
          ? 'This message has no unsubscribe link.'
          : err instanceof RpcError
            ? `Couldn't unsubscribe: ${err.message}`
            : "Couldn't reach the engine.",
      );
    } finally {
      setBusy(false);
    }
  };
  return { available, busy, run };
}
