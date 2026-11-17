// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

// The engine's mail intents (`threads.*`) and undo log for the web. An intent takes threads
// of any accounts; the engine decides what it means on each (Gmail labels or folders, the
// archive fallbacks), queues the tasks and keeps an undo entry, and answers with the Thread
// rows as the local phases left them. Those rows go into the cache, so a relayed browser
// shows the change before its live queries catch up; nothing is guessed.
import type { QueryClient } from '@tanstack/react-query';
import { engineKeys, EVERY_ACCOUNT } from './keys';
import type { LiveSnapshot } from './live';
import type { SearchResult } from './rpc';
import type { ThreadJSON } from './types';

/** The intents `useThreadActions` calls. */
export type ThreadIntent =
  | 'threads.archive'
  | 'threads.trash'
  | 'threads.spam'
  | 'threads.notSpam'
  | 'threads.move'
  | 'threads.label'
  | 'threads.markRead'
  | 'threads.star';

/**
 * Writes the engine's Thread rows over the cached ones (lists, the open thread, search
 * results), keeping each row's summary. Nothing is inserted, dropped or re-sorted: the live
 * queries' next change does that.
 */
export function applyModels(qc: QueryClient, models: readonly unknown[] | undefined) {
  const threads = (models ?? []).filter(
    (m): m is ThreadJSON => !!m && (m as { __cls?: string }).__cls === 'Thread',
  );
  if (threads.length === 0) return;
  const byKey = new Map(threads.map((t) => [`${t.aid}:${t.id}`, t]));
  const patch = (rows: ThreadJSON[]) => {
    let changed = false;
    const out = rows.map((r) => {
      const next = byKey.get(`${r.aid}:${r.id}`);
      if (!next) return r;
      changed = true;
      return { ...next, summary: r.summary };
    });
    return changed ? out : rows;
  };
  const accounts = new Set(threads.map((t) => t.aid));
  for (const aid of [...accounts, EVERY_ACCOUNT]) {
    for (const query of qc.getQueryCache().findAll({ queryKey: engineKeys.threadLists(aid) })) {
      qc.setQueryData<LiveSnapshot<ThreadJSON>>(query.queryKey, (list) =>
        list ? { ...list, rows: patch(list.rows) } : list,
      );
    }
  }
  for (const t of threads) {
    qc.setQueryData<ThreadJSON | null>(engineKeys.thread(t.aid, t.id), (old) =>
      old ? { ...t, summary: old.summary } : old,
    );
  }
  for (const query of qc.getQueryCache().findAll({ queryKey: engineKeys.searchAll() })) {
    qc.setQueryData<SearchResult & { truncated?: boolean }>(
      query.queryKey,
      (r) => r && { ...r, threads: patch(r.threads) },
    );
  }
}
