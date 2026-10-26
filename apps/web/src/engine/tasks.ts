// Task builders for what the web queues itself: drafts, metadata, and marking a thread read
// as it opens (no undo entry). Other thread changes are the engine's intents
// (`useIntents`). The JSON matches what the desktop's Task subclasses serialise, so the
// engine's task factory accepts it unchanged.
import { generateId } from './model';
import type {
  ChangeUnreadTaskJSON,
  DestroyDraftTaskJSON,
  MessageJSON,
  SendDraftTaskJSON,
  SyncbackDraftTaskJSON,
  SyncbackMetadataTaskJSON,
  TaskBaseJSON,
} from './types';

export function base(accountId: string): Omit<TaskBaseJSON, '__cls'> {
  return { id: generateId(), aid: accountId, v: 0, status: 'local' };
}

export interface MailTarget {
  accountId: string;
  threadIds: string[];
  messageIds?: string[];
  source?: string;
  canBeUndone?: boolean;
}

export function mail({
  accountId,
  threadIds,
  messageIds = [],
  source,
  canBeUndone = true,
}: MailTarget) {
  return { ...base(accountId), threadIds, messageIds, source, canBeUndone, isUndo: false };
}

export function changeUnread(t: MailTarget & { unread: boolean }): ChangeUnreadTaskJSON {
  return { ...mail(t), __cls: 'ChangeUnreadTask', unread: t.unread };
}

export function syncbackDraft(draft: MessageJSON): SyncbackDraftTaskJSON {
  return { ...base(draft.aid), __cls: 'SyncbackDraftTask', headerMessageId: draft.hMsgId, draft };
}

export function sendDraft(draft: MessageJSON, opts: { silent?: boolean } = {}): SendDraftTaskJSON {
  return {
    ...base(draft.aid),
    __cls: 'SendDraftTask',
    headerMessageId: draft.hMsgId,
    draft,
    silent: opts.silent ?? false,
  };
}

export function destroyDraft(accountId: string, messageIds: string[]): DestroyDraftTaskJSON {
  return { ...base(accountId), __cls: 'DestroyDraftTask', messageIds };
}

export function syncbackMetadata(args: {
  accountId: string;
  pluginId: string;
  modelId: string;
  modelClassName: string;
  modelHeaderMessageId?: string;
  value: unknown;
}): SyncbackMetadataTaskJSON {
  const { accountId, ...rest } = args;
  return { ...base(accountId), __cls: 'SyncbackMetadataTask', ...rest };
}
