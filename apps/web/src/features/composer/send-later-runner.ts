// Sends scheduled drafts when the engine reports their `send-later` metadata expired, as the
// desktop's send-later plugin does (handleMetadataExpiration): clear the metadata, then send.
// TODO(engine): this only runs while a tab is open. DCP stage 4's expiration executor sends
// due drafts itself (gate this on `expiration-executor`); with no desktop running, that also
// needs stage 11's detached engine.
import { toast } from 'sonner';
import {
  ensureConnected,
  nowSeconds,
  RpcError,
  RpcErrorCode,
  tasks,
  useDeltas,
  useEngine,
  type EngineClient,
  type MessageJSON,
} from '@/engine';
import { queueTask } from './engine-calls';
import { SEND_LATER_PLUGIN_ID } from './send-later';

interface SendLaterValue {
  expiration?: number | null;
}

function expiredSendLater(m: MessageJSON): boolean {
  const entry = m.metadata?.find((x) => x.pluginId === SEND_LATER_PLUGIN_ID);
  const exp = (entry?.value as SendLaterValue | undefined)?.expiration;
  return typeof exp === 'number' && exp <= nowSeconds();
}

function withoutSendLater(m: MessageJSON): MessageJSON {
  return {
    ...m,
    metadata: (m.metadata ?? []).map((x) =>
      x.pluginId === SEND_LATER_PLUGIN_ID ? { ...x, v: x.v + 1, value: { expiration: null } } : x,
    ),
  };
}

/** One tab at a time: the others find the draft gone once the first has queued the send. */
function exclusive(name: string, fn: () => Promise<void>): Promise<void> {
  if (typeof navigator === 'undefined' || !navigator.locks) return fn();
  return navigator.locks.request(name, fn);
}

async function sendExpired(client: EngineClient, m: MessageJSON) {
  await ensureConnected(client);
  if (!m.draft) {
    await queueTask(
      client,
      tasks.syncbackMetadata({
        accountId: m.aid,
        pluginId: SEND_LATER_PLUGIN_ID,
        modelId: m.id,
        modelClassName: 'Message',
        modelHeaderMessageId: m.hMsgId,
        value: { expiration: null },
      }),
    );
    return;
  }
  // Delta JSON carries no body; the send needs the stored one.
  const current = await client
    .call('messages.get', { accountId: m.aid, id: m.id, body: true })
    .catch((err: unknown) => {
      if (err instanceof RpcError && err.code === RpcErrorCode.NotFound) return null;
      throw err;
    });
  // Sent, deleted or rescheduled since the delta.
  if (!current || !current.message.draft || !expiredSendLater(current.message)) return;
  const draft = withoutSendLater({ ...current.message, body: current.body ?? '' });
  // Saving the cleared metadata first is what tells another tab not to send it too.
  await queueTask(client, tasks.syncbackDraft(draft));
  await queueTask(client, tasks.sendDraft(draft));
  toast.success(`Sent your scheduled message “${draft.subject || '(no subject)'}”`);
}

export function useSendLaterRunner() {
  const client = useEngine();
  useDeltas(
    (d) => {
      if (d.type !== 'metadata-expiration') return;
      for (const m of d.modelJSONs as MessageJSON[]) {
        if (!expiredSendLater(m)) continue;
        void exclusive(`tern-send-later:${m.id}`, () => sendExpired(client, m)).catch(() =>
          toast.error(`Couldn't send the scheduled message “${m.subject || '(no subject)'}”`),
        );
      }
    },
    { modelClass: 'Message' },
  );
}
