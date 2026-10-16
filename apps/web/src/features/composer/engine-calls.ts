import { ensureConnected, type EngineClient, type TaskJSON } from '@/engine';

/** Queues a task without the optimistic thread update in `useQueueTask` (drafts have none). */
export async function queueTask(client: EngineClient, task: TaskJSON): Promise<TaskJSON> {
  await ensureConnected(client);
  return client.call('task.queue', { accountId: task.aid, task });
}
