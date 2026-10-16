// Builds a window's starting draft: blank, a reply or forward of a message, or a saved draft.
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
import { draftFromMessage, forwardDraft, newDraft, replyDraft, type DraftState } from './draft';
import { defaultIdentity, ownEmails, replyIdentity } from './identities';
import { withSignature } from './signature';
import type { ComposerWindow } from './store';

export interface DraftSource {
  initial: { draft: DraftState; existing: boolean } | null;
  /** The message being replied to or forwarded, kept current as its body arrives. */
  source?: MessageJSON;
  error?: string;
}

function pickSource(messages: MessageJSON[] | undefined, messageId?: string) {
  if (!messages?.length) return undefined;
  if (messageId) return messages.find((m) => m.id === messageId);
  const sorted = [...messages].sort(compareMessages);
  return sorted.filter((m) => !m.draft).at(-1) ?? sorted.at(-1);
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

  const [initial, setInitial] = useState<DraftSource['initial']>(null);

  let error: string | undefined;
  if (accounts.error) error = "Couldn't reach the engine.";
  else if (list && list.length === 0) error = 'Add an account to send mail.';
  else if (replying && !win.threadId) error = 'Nothing to reply to.';
  else if (replying && messages.isSuccess && !source) error = "Couldn't find that message.";
  else if (messages.error) error = "Couldn't load the message.";
  else if (saved.error) error = saved.error.message;

  // Built once, during render, as soon as its inputs are known (React's "adjust state on
  // prop change" pattern); later changes to the inputs must not reset the user's edits.
  if (!initial && !error && account && list) {
    let next: DraftState | null = null;
    if (win.mode === 'new') {
      next = newDraft(defaultIdentity(account), win, account);
    } else if (win.mode === 'edit-draft') {
      if (saved.data?.body != null) next = draftFromMessage(saved.data);
    } else if (source) {
      const identity = replyIdentity(account, source);
      next =
        win.mode === 'forward'
          ? forwardDraft({ message: source, identity })
          : replyDraft({ message: source, identity, own: ownEmails(list), mode: win.mode });
    }
    if (next) {
      const existing = win.mode === 'edit-draft';
      setInitial({ draft: existing ? next : withSignature(next), existing });
    }
  }

  return { initial, source, error };
}
