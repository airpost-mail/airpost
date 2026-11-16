// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

/*
Triage: the engine's triage worker files a `airpost-ai-triage` value on
each new inbox thread of an opted-in account. This module reads that value, writes the user's
corrections (`source: 'user'`, which triage never replaces), and makes the `ai.triage.run` and
`ai.triage.list` calls. The value stays on this computer: airpost-net never syncs the plugin id.
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
import { AiError, providerParams, toAiError } from './client';

export const PLUGIN_ID = 'airpost-ai-triage';
// How far back "Triage inbox" goes, as `airpost ai triage` does by default.
export const TRIAGE_SINCE_DAYS = 7;
// The "Needs reply" list shows at most this many threads (airpost-ipc READ_MAX_LIMIT).
export const LIST_LIMIT = 500;

export type TriageCategory =
  | 'needs-reply'
  | 'important'
  | 'newsletter'
  | 'notification'
  | 'promotion'
  | 'other';

// airpost-ai `Category::ALL`, in its order.
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

function seconds(date: Date | null | undefined) {
  return date instanceof Date ? date.getTime() / 1000 : 0;
}

/**
 * Whether the user replied since the value said "needs reply": mail sent after the last mail
 * came in, or, for a value the user filed, after filing it. As airpost-store `triaged_threads`.
 */
export function answered(value: TriageValue, thread: Thread | null | undefined) {
  if (!thread) return false;
  const sent = seconds(thread.lastMessageSentTimestamp);
  if (!sent) return false;
  if (value.source === 'user' && typeof value.at === 'number' && value.at > sent) return false;
  return sent >= seconds(thread.lastMessageReceivedTimestamp);
}

/** The chip a thread shows: only needs-reply and important, to keep the list quiet. */
export function chipCategory(
  value: TriageValue | null,
  thread?: Thread | null
): 'needs-reply' | 'important' | null {
  if (!value) return null;
  const needsReply = value.category === 'needs-reply' || value.needsReply;
  if (needsReply && !answered(value, thread)) return 'needs-reply';
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

/** Whether the account gets triage: the engine is connected, triage is on and the account opted in. */
export function triageShownFor(accountId: string) {
  if (!Engine.connected) return false;
  const settings = AI.readSettings();
  return settings.features.triage.enabled && AI.accountEnabled(settings, accountId);
}

export interface TriageRun {
  /** Resolves with the job once the engine queued it. */
  started: Promise<AiTriageRunResult>;
  /**
   * Resolves when every thread is done; rejects when the job stopped early (`stopped`), was
   * cancelled (`cancelled`) or the engine went away (`engine-disconnected`).
   */
  finished: Promise<void>;
  cancel(): void;
  /** Stops watching the job, which runs on in the engine. */
  dispose(): void;
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
  // Stop was clicked before ai.triage.run answered: the job is cancelled once its id comes.
  let cancelled = false;
  // Assigned by the Promise executor, which runs synchronously.
  let finish!: () => void;
  let fail!: (err: AiError) => void;
  const finished = new Promise<void>((resolve, reject) => {
    finish = resolve;
    fail = reject;
  });
  // The first progress can arrive before the reply's continuation runs: keep it until then.
  const early: AiTriageProgressParams[] = [];
  const seen = (p: AiTriageProgressParams) => {
    if (p.stopped) {
      fail(new AiError({ key: 'stopped', message: `triage stopped after ${p.done} threads` }));
      return;
    }
    onProgress(p.done, p.total);
    if (p.done >= p.total) finish();
  };
  const subscription = Engine.subscribe({ methods: ['ai.triage.progress'] }, (n) => {
    const p = n.params as AiTriageProgressParams;
    if (!p || typeof p.jobId !== 'string') return;
    if (jobId === null) early.push(p);
    else if (p.jobId === jobId) seen(p);
  });
  // The connection dropped: no more progress comes for this job.
  const gone = () =>
    fail(new AiError({ key: 'engine-disconnected', message: 'the engine went away' }));
  Engine.events.on('gone', gone);
  const dispose = () => {
    subscription.dispose();
    Engine.events.removeListener('gone', gone);
  };
  const started = (async () => {
    try {
      if (!Engine.connected) {
        throw new AiError({ key: 'engine-disconnected', message: 'the engine is not connected' });
      }
      const settings = AI.readSettings();
      const provider = AI.providerFor(settings, 'triage');
      if (!settings.features.triage.enabled || !provider) {
        throw new AiError({ key: 'not-configured', message: 'no model is set up for triage' });
      }
      const result = await Engine.request<AiTriageRunResult>('ai.triage.run', {
        accountId,
        sinceDays: TRIAGE_SINCE_DAYS,
        ...(await providerParams(provider)),
      });
      jobId = result.jobId;
      if (cancelled) {
        Engine.request('ai.cancel', { jobId }).catch(() => {});
        return result;
      }
      for (const p of early.splice(0)) if (p.jobId === jobId) seen(p);
      if (result.queued === 0) finish();
      return result;
    } catch (err) {
      const e = toAiError(err);
      fail(e);
      throw e;
    }
  })();
  finished.then(dispose, dispose);
  // The caller may watch only one of the two.
  started.catch(() => {});
  finished.catch(() => {});
  return {
    started,
    finished,
    cancel() {
      cancelled = true;
      if (jobId) Engine.request('ai.cancel', { jobId }).catch(() => {});
      fail(new AiError({ key: 'cancelled' }));
    },
    dispose,
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
