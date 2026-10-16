// Which messages of a thread start expanded: unread ones, drafts and the last, judged on
// the messages as they were when the thread opened (marking read must not collapse them).
// Messages that arrive while the thread is open start expanded.
import { useState } from 'react';
import type { MessageJSON } from '@/engine';

interface Snapshot {
  ids: ReadonlySet<string>;
  open: ReadonlySet<string>;
}

export function initialExpansion(messages: MessageJSON[]): Snapshot {
  const last = messages[messages.length - 1];
  return {
    ids: new Set(messages.map((m) => m.id)),
    open: new Set(messages.filter((m) => m.unread || m.draft || m === last).map((m) => m.id)),
  };
}

export function useExpansion(messages: MessageJSON[] | undefined) {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [overrides, setOverrides] = useState<Record<string, boolean>>({});
  if (messages && messages.length > 0 && !snapshot) setSnapshot(initialExpansion(messages));

  const isExpanded = (m: MessageJSON) =>
    overrides[m.id] ?? (!snapshot || !snapshot.ids.has(m.id) || snapshot.open.has(m.id));
  const list = messages ?? [];
  const allExpanded = list.length > 0 && list.every(isExpanded);

  return {
    isExpanded,
    allExpanded,
    toggle: (m: MessageJSON) => setOverrides((o) => ({ ...o, [m.id]: !isExpanded(m) })),
    expand: (ms: MessageJSON[]) =>
      setOverrides((o) => ({ ...o, ...Object.fromEntries(ms.map((m) => [m.id, true])) })),
    setAll: (expanded: boolean) =>
      setOverrides(
        Object.fromEntries(list.map((m, i) => [m.id, expanded || i === list.length - 1])),
      ),
  };
}

/** Splits messages into runs, folding 3+ consecutive collapsed ones into one group. */
export type MessageRun =
  { kind: 'message'; message: MessageJSON } | { kind: 'folded'; messages: MessageJSON[] };

export function foldRuns(
  messages: MessageJSON[],
  isExpanded: (m: MessageJSON) => boolean,
): MessageRun[] {
  const out: MessageRun[] = [];
  let pending: MessageJSON[] = [];
  const flush = () => {
    if (pending.length >= 3) out.push({ kind: 'folded', messages: pending });
    else out.push(...pending.map((message) => ({ kind: 'message' as const, message })));
    pending = [];
  };
  messages.forEach((m, i) => {
    // The first message always shows, so the thread reads from its start.
    if (i > 0 && !isExpanded(m)) pending.push(m);
    else {
      flush();
      out.push({ kind: 'message', message: m });
    }
  });
  flush();
  return out;
}
