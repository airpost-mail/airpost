// A window's starting draft: a new draft, reply or forward the engine builds (`draft.create`,
// not saved: the composer saves it once edited), or a saved draft.
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import {
  compareMessages,
  ensureConnected,
  RpcError,
  RpcErrorCode,
  useAccounts,
  useDeltas,
  useEngine,
  useMessages,
  type AccountJSON,
  type MessageJSON,
} from '@/engine';
import { draftFromMessage, type DraftState } from './draft';
import { textToHtml } from './html';
import { signed, withSignature } from './signature';
import type { ComposerWindow } from './store';

export interface DraftSource {
  initial: { draft: DraftState; existing: boolean } | null;
  error?: string;
}

function pickSource(messages: MessageJSON[] | undefined, messageId?: string) {
  if (!messages?.length) return undefined;
  if (messageId) return messages.find((m) => m.id === messageId);
  const sorted = [...messages].sort(compareMessages);
  return sorted.filter((m) => !m.draft && !m.hidden).at(-1) ?? sorted.at(-1);
}

function useSavedDraft(accountId: string | undefined, draftId: string | undefined) {
  const client = useEngine();
  const qc = useQueryClient();
  const key = ['composer', 'draft', accountId ?? '', draftId ?? ''];
  const query = useQuery({
    queryKey: key,
    queryFn: async (): Promise<MessageJSON> => {
      await ensureConnected(client);
      const { message, body } = await client
        .call('messages.get', { accountId: accountId!, id: draftId!, body: true })
        .catch((err: unknown) => {
          if (err instanceof RpcError && err.code === RpcErrorCode.NotFound) {
            throw new Error('That draft no longer exists.');
          }
          throw err;
        });
      // A draft saved on another device may not be downloaded yet; it arrives as a delta.
      if (body == null)
        void client.call('bodies.fetch', { accountId: accountId!, ids: [message.id] });
      return body == null ? message : { ...message, body };
    },
    enabled: !!accountId && !!draftId,
    // Never reuse a copy: the draft may have been saved since, and editing a stale copy
    // would overwrite the newer text on the next autosave.
    staleTime: 0,
    gcTime: 0,
    retry: false,
  });
  useDeltas(
    (d) => {
      const hit = (d.modelJSONs as MessageJSON[]).find((m) => m.id === draftId);
      if (hit && d.type === 'persist' && hit.body != null) qc.setQueryData(key, hit);
    },
    { accountId, modelClass: 'Message' },
  );
  return query;
}

const ENGINE_SIGNATURE = /<signature id="([^"]*)">([\s\S]*?)<\/signature>/;

/**
 * The composer's copy of a draft the engine built. The engine signs it with the signatures it
 * keeps, which become the editor's signature block; without one, this browser's is added.
 */
function fromCreated(m: MessageJSON): DraftState {
  const body = m.body ?? '';
  const sig = ENGINE_SIGNATURE.exec(body);
  if (!sig) return withSignature(draftFromMessage(m));
  return signed(draftFromMessage({ ...m, body: body.replace(sig[0], '') }), sig[1], sig[2]);
}

/**
 * The engine's new draft for the window: its addresses, subject, quote, signature, automatic
 * Cc/Bcc and, for a forward, the original's files. A reply or forward waits for the source
 * message's body, which the engine quotes; `useMessages` downloads it.
 */
function useCreatedDraft(win: ComposerWindow, accountId: string | undefined, source?: MessageJSON) {
  const client = useEngine();
  const kind = win.mode === 'edit-draft' ? null : win.mode;
  const ready = !!accountId && (kind === 'new' || source?.body != null);
  return useQuery({
    queryKey: ['composer', 'create', win.key],
    queryFn: async (): Promise<MessageJSON> => {
      await ensureConnected(client);
      const fields: Record<string, unknown> = {};
      if (kind === 'new') {
        for (const k of ['to', 'cc', 'bcc', 'subject'] as const) if (win[k]) fields[k] = win[k];
        if (win.body) fields.body = textToHtml(win.body);
      }
      return client.call('draft.create', {
        kind: kind!,
        accountId,
        messageId: kind === 'new' ? undefined : source!.id,
        fields,
        save: false,
        html: true,
        tz: Intl.DateTimeFormat().resolvedOptions().timeZone,
      });
    },
    enabled: !!kind && ready,
    staleTime: Infinity,
    gcTime: 0,
    retry: false,
  });
}

export function useDraftSource(win: ComposerWindow, fallbackAccountId?: string): DraftSource {
  const accounts = useAccounts();
  const list = accounts.data;
  const accountId =
    win.accountId ??
    (list?.some((a) => a.id === fallbackAccountId) ? fallbackAccountId : list?.[0]?.id);
  const account: AccountJSON | undefined = list?.find((a) => a.id === accountId);

  const replying = win.mode === 'reply' || win.mode === 'reply-all' || win.mode === 'forward';
  const messages = useMessages(accountId, replying ? win.threadId : undefined);
  const saved = useSavedDraft(accountId, win.mode === 'edit-draft' ? win.draftId : undefined);
  const source = replying ? pickSource(messages.data, win.messageId) : undefined;
  const created = useCreatedDraft(win, account?.id, source);

  const [initial, setInitial] = useState<DraftSource['initial']>(null);

  let error: string | undefined;
  if (accounts.error) error = "Couldn't reach the engine.";
  else if (list && list.length === 0) error = 'Add an account to send mail.';
  else if (replying && !win.threadId) error = 'Nothing to reply to.';
  else if (replying && messages.isSuccess && !source) error = "Couldn't find that message.";
  else if (messages.error) error = "Couldn't load the message.";
  else if (saved.error) error = saved.error.message;
  else if (created.error) error = `Couldn't start the draft: ${created.error.message}`;

  // Set once, during render, as soon as it is known (React's "adjust state on prop change"
  // pattern); later changes to the inputs must not reset the user's edits.
  if (!initial && !error && account && list) {
    if (win.mode === 'edit-draft') {
      if (saved.data?.body != null)
        setInitial({ draft: draftFromMessage(saved.data), existing: true });
    } else if (created.data) {
      setInitial({ draft: fromCreated(created.data), existing: false });
    }
  }

  return { initial, error };
}
