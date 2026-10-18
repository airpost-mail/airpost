import {
  ensureConnected,
  type Delta,
  type EngineClient,
  type MessageJSON,
  type TaskJSON,
} from '@/engine';
import { SEND_LATER_PLUGIN_ID } from './send-later';

/** Queues a task without the optimistic thread update in `useQueueTask` (drafts have none). */
export async function queueTask(client: EngineClient, task: TaskJSON): Promise<TaskJSON> {
  await ensureConnected(client);
  return client.call('task.queue', { accountId: task.aid, task });
}

export type EngineSendOutcome =
  { kind: 'task'; task: TaskJSON } | { kind: 'failed'; reason: string } | { kind: 'timeout' };

/**
 * Follows the send the engine's expiration executor starts once a draft's `send-later`
 * metadata comes due: its SendDraftTask finishing, or the executor giving up with `error` on
 * the metadata. `stop()` ends the watch (Undo); `done` then never settles.
 */
export function watchEngineSend(
  client: EngineClient,
  draft: { id: string; headerMessageId: string },
  timeoutMs: number,
): { done: Promise<EngineSendOutcome>; stop: () => void } {
  let stop = () => {};
  const done = new Promise<EngineSendOutcome>((resolve) => {
    const finish = (o: EngineSendOutcome) => {
      stop();
      resolve(o);
    };
    const timer = setTimeout(() => finish({ kind: 'timeout' }), timeoutMs);
    const off = client.onNotification((n) => {
      if (n.method !== 'delta') return;
      const d = n.params as Delta;
      if (d.modelClass === 'Task') {
        const task = (d.modelJSONs as TaskJSON[]).find(
          (t) => t.__cls === 'SendDraftTask' && t.headerMessageId === draft.headerMessageId,
        );
        if (task && (task.status === 'complete' || task.status === 'cancelled'))
          finish({ kind: 'task', task });
      } else if (d.modelClass === 'Message') {
        const m = (d.modelJSONs as MessageJSON[]).find(
          (x) => x.id === draft.id || x.hMsgId === draft.headerMessageId,
        );
        const value = m?.metadata?.find((x) => x.pluginId === SEND_LATER_PLUGIN_ID)?.value as
          { error?: unknown; isUndoSend?: unknown } | undefined;
        // An `error` left from an earlier scheduled send has no `isUndoSend`.
        if (m?.draft && value?.isUndoSend === true && typeof value.error === 'string')
          finish({ kind: 'failed', reason: value.error });
      }
    });
    stop = () => {
      clearTimeout(timer);
      off();
    };
  });
  return { done, stop: () => stop() };
}
