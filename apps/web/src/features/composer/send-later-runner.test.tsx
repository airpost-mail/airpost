import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import {
  EngineFeature,
  EngineProvider,
  ensureConnected,
  MockEngine,
  nowSeconds,
  tasks,
  type TaskJSON,
} from '@/engine';
import { MOCK_FEATURES } from '@/engine/mock/engine';
import { useSendLaterRunner } from './send-later-runner';
import { SEND_LATER_PLUGIN_ID } from './send-later';

function Runner() {
  useSendLaterRunner();
  return null;
}

async function scheduleDue(features: readonly string[]) {
  const engine = new MockEngine({ latency: 0, features });
  const queued: TaskJSON[] = [];
  const call = engine.call.bind(engine);
  vi.spyOn(engine, 'call').mockImplementation(((method: string, params: unknown) => {
    if (method === 'task.queue') queued.push((params as { task: TaskJSON }).task);
    return call(method as never, params as never);
  }) as typeof engine.call);
  render(
    <QueryClientProvider client={new QueryClient()}>
      <EngineProvider client={engine}>
        <Runner />
      </EngineProvider>
    </QueryClientProvider>,
  );
  await ensureConnected(engine);
  const draft = [...engine.store.messages.values()].find((m) => m.draft)!;
  await engine.call('task.queue', {
    accountId: draft.aid,
    task: tasks.syncbackMetadata({
      accountId: draft.aid,
      pluginId: SEND_LATER_PLUGIN_ID,
      modelId: draft.id,
      modelClassName: 'Message',
      modelHeaderMessageId: draft.hMsgId,
      value: { expiration: nowSeconds() - 1 },
    }),
  });
  return queued;
}

describe('useSendLaterRunner', () => {
  it('sends a due draft for an engine without the expiration executor', async () => {
    const queued = await scheduleDue(MOCK_FEATURES);
    await waitFor(() => expect(queued.some((t) => t.__cls === 'SendDraftTask')).toBe(true));
  });

  it('leaves due drafts to an engine with the expiration executor', async () => {
    const queued = await scheduleDue([...MOCK_FEATURES, EngineFeature.ExpirationExecutor]);
    // Long enough for the expiration delta and a send it would have queued.
    await new Promise((r) => setTimeout(r, 300));
    expect(queued.map((t) => t.__cls)).toEqual(['SyncbackMetadataTask']);
  });
});
