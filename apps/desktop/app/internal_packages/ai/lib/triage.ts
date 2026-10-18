// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Triage (docs/ai-plan.md section 5): the engine's triage worker files a `tern-ai-triage` value on
each new inbox thread of an opted-in account. This module reads that value, writes the user's
corrections (`source: 'user'`, which triage never replaces), and makes the `ai.triage.run` and
`ai.triage.list` calls. The value stays on this computer: ms-net never syncs the plugin id.
*/
import {
  AI,
  Actions,
  Engine,
  SyncbackMetadataTask,
  Thread,
  localized,
  AiTriageListResult,
  AiTriageProgressParams,
  AiTriageRunResult,
} from 'mailspring-exports';
import { AiError, configFor, toAiError } from './ai-client';

export const PLUGIN_ID = 'tern-ai-triage';
// ms-ipc `feature::AI_TRIAGE`.
export const TRIAGE_ENGINE_FEATURE = 'ai-triage';
// How far back "Triage inbox" goes, as `tern ai triage` does by default.
export const TRIAGE_SINCE_DAYS = 7;
// The "Needs reply" list shows at most this many threads (ms-ipc READ_MAX_LIMIT).
export const LIST_LIMIT = 500;

export type TriageCategory =
  | 'needs-reply'
  | 'important'
  | 'newsletter'
  | 'notification'
  | 'promotion'
  | 'other';

// ms-ai `Category::ALL`, in its order.
export const CATEGORIES: TriageCategory[] = [
  'needs-reply',
  'important',
  'newsletter',
  'notification',
  'promotion',
  'other',
];

export interface TriageValue {
  category: TriageCategory;
  needsReply?: boolean;
  reason?: string;
  messageId?: string;
  model?: string;
  v?: number;
  at?: number;
  source?: 'user';
}

export function triageOf(thread: Thread | null | undefined): TriageValue | null {
  if (!thread || typeof thread.metadataForPluginId !== 'function') return null;
  const value = thread.metadataForPluginId(PLUGIN_ID) as TriageValue | null;
  if (!value || !CATEGORIES.includes(value.category)) return null;
  return value;
}

/** The chip a thread shows: only needs-reply and important, to keep the list quiet. */
export function chipCategory(value: TriageValue | null): 'needs-reply' | 'important' | null {
  if (!value) return null;
  if (value.category === 'needs-reply' || value.needsReply) return 'needs-reply';
  if (value.category === 'important') return 'important';
  return null;
}

export function categoryLabel(category: TriageCategory) {
  switch (category) {
    case 'needs-reply':
      return localized('Needs reply');
    case 'important':
      return localized('Important');
    case 'newsletter':
      return localized('Newsletter');
    case 'notification':
      return localized('Notification');
    case 'promotion':
      return localized('Promotion');
    default:
      return localized('Other');
  }
}

/** Files the user's category for `thread`. The model's reason no longer applies, so it goes. */
export function correctCategory(thread: Thread, category: TriageCategory) {
  const { reason, ...previous } = triageOf(thread) || ({} as Partial<TriageValue>);
  const value: TriageValue = {
    ...previous,
    category,
    needsReply: category === 'needs-reply',
    source: 'user',
    at: Math.floor(Date.now() / 1000),
  };
  Actions.queueTask(SyncbackMetadataTask.forSaving({ model: thread, pluginId: PLUGIN_ID, value }));
}

/** Whether the account gets triage: the engine triages, triage is on and the account opted in. */
export function triageShownFor(accountId: string) {
  if (!Engine.has(TRIAGE_ENGINE_FEATURE)) return false;
  const settings = AI.readSettings();
  return settings.features.triage.enabled && AI.accountEnabled(settings, accountId);
}

export interface TriageRun {
  /** Resolves with the job once the engine queued it. */
  started: Promise<AiTriageRunResult>;
  /** Resolves when every thread is done. */
  finished: Promise<void>;
  cancel(): void;
}

/**
 * "Triage inbox": `ai.triage.run` over the account's inbox threads of the last week, with the
 * triage provider's config (and key) so a keyed provider works before the engine holds keys.
 * `onProgress` gets each `ai.triage.progress`.
 */
export function triageInbox(
  accountId: string,
  onProgress: (done: number, total: number) => void = () => {}
): TriageRun {
  let jobId: string | null = null;
  let finish: () => void;
  let fail: (err: AiError) => void;
  const finished = new Promise<void>((resolve, reject) => {
    finish = resolve;
    fail = reject;
  });
  // The first progress can arrive before the reply's continuation runs: keep it until then.
  const early: AiTriageProgressParams[] = [];
  const seen = (p: AiTriageProgressParams) => {
    onProgress(p.done, p.total);
    if (p.done >= p.total) finish();
  };
  const subscription = Engine.subscribe({ methods: ['ai.triage.progress'] }, (n) => {
    const p = n.params as AiTriageProgressParams;
    if (!p || typeof p.jobId !== 'string') return;
    if (jobId === null) early.push(p);
    else if (p.jobId === jobId) seen(p);
  });
  const started = (async () => {
    try {
      if (!Engine.has(TRIAGE_ENGINE_FEATURE)) {
        throw new AiError({ key: 'engine-unavailable', message: 'no ai.triage.run' });
      }
      const settings = AI.readSettings();
      const provider = AI.providerFor(settings, 'triage');
      if (!settings.features.triage.enabled || !provider) {
        throw new AiError({ key: 'not-configured', message: 'no model is set up for triage' });
      }
      const result = await Engine.request<AiTriageRunResult>('ai.triage.run', {
        accountId,
        sinceDays: TRIAGE_SINCE_DAYS,
        config: await configFor(provider),
      });
      jobId = result.jobId;
      for (const p of early.splice(0)) if (p.jobId === jobId) seen(p);
      if (result.queued === 0) finish();
      return result;
    } catch (err) {
      const e = toAiError(err);
      fail(e);
      throw e;
    }
  })();
  finished.then(
    () => subscription.dispose(),
    () => subscription.dispose()
  );
  // The caller may watch only one of the two.
  started.catch(() => {});
  finished.catch(() => {});
  return {
    started,
    finished,
    cancel() {
      if (jobId) Engine.request('ai.cancel', { jobId }).catch(() => {});
      fail(new AiError({ key: 'cancelled' }));
    },
  };
}

/** The opted-in accounts' inbox threads triaged `category`, newest first. */
export async function listTriaged(accountIds: string[], category: TriageCategory) {
  try {
    return await Engine.request<AiTriageListResult>('ai.triage.list', {
      accountIds,
      category,
      limit: LIST_LIMIT,
    });
  } catch (err) {
    throw toAiError(err);
  }
}
