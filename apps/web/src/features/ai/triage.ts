// Triage results: the engine files a `tern-ai-triage` value on
// each new inbox thread of an opted-in account. The web reads it from the thread's metadata
// and files the user's corrections (`source: 'user'`, which triage never replaces) as the
// desktop does. The value stays on the engine's computer: tern-net never syncs the plugin id.
// Module imports, not '@/engine': the mock engine loads this file too.
import { nowSeconds } from '@/engine/model';
import { syncbackMetadata } from '@/engine/tasks';
import type { TaskJSON, ThreadJSON } from '@/engine/types';

export const PLUGIN_ID = 'tern-ai-triage';

export type TriageCategory =
  'needs-reply' | 'important' | 'newsletter' | 'notification' | 'promotion' | 'other';

/** tern-ai `Category::ALL`, in its order. */
export const CATEGORIES: TriageCategory[] = [
  'needs-reply',
  'important',
  'newsletter',
  'notification',
  'promotion',
  'other',
];

export const CATEGORY_LABEL: Record<TriageCategory, string> = {
  'needs-reply': 'Needs reply',
  important: 'Important',
  newsletter: 'Newsletter',
  notification: 'Notification',
  promotion: 'Promotion',
  other: 'Other',
};

export interface TriageValue {
  category: TriageCategory;
  needsReply?: boolean;
  reason?: string | null;
  messageId?: string;
  model?: string;
  v?: number;
  at?: number;
  source?: 'user';
}

export function triageOf(thread: ThreadJSON | null | undefined): TriageValue | null {
  const value = thread?.metadata?.find((m) => m.pluginId === PLUGIN_ID)?.value as
    TriageValue | null | undefined;
  if (!value || typeof value !== 'object' || !CATEGORIES.includes(value.category)) return null;
  return value;
}

/**
 * Whether the user replied since the value said "needs reply": mail sent after the last
 * mail came in, or, for a value the user filed, after filing it. As tern-store's
 * `triaged_threads` and the desktop's chip.
 */
export function answered(value: TriageValue, thread: ThreadJSON): boolean {
  const sent = thread.lmst;
  if (!sent) return false;
  if (value.source === 'user' && typeof value.at === 'number' && value.at > sent) return false;
  return sent >= thread.lmrt;
}

/** The chip a thread shows: needs-reply and important only, to keep the list quiet. */
export function chipCategory(
  value: TriageValue | null,
  thread: ThreadJSON,
): 'needs-reply' | 'important' | null {
  if (!value) return null;
  if ((value.category === 'needs-reply' || value.needsReply) && !answered(value, thread))
    return 'needs-reply';
  return value.category === 'important' ? 'important' : null;
}

/** The user's category for `thread`. The model's reason no longer applies, so it goes. */
export function correctionTask(thread: ThreadJSON, category: TriageCategory): TaskJSON {
  const { reason: _reason, ...previous } = triageOf(thread) ?? ({} as Partial<TriageValue>);
  const value: TriageValue = {
    ...previous,
    category,
    needsReply: category === 'needs-reply',
    source: 'user',
    at: nowSeconds(),
  };
  return syncbackMetadata({
    accountId: thread.aid,
    pluginId: PLUGIN_ID,
    modelId: thread.id,
    modelClassName: 'thread',
    value,
  });
}
