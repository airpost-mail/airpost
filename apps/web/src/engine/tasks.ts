// Task builders. The JSON matches what the desktop's Task subclasses serialise, so the
// engine's task factory accepts it unchanged.
import { generateId } from './model';
import type {
  ChangeFolderTaskJSON,
  ChangeLabelsTaskJSON,
  ChangeStarredTaskJSON,
  ChangeUnreadTaskJSON,
  DestroyCategoryTaskJSON,
  DestroyDraftTaskJSON,
  ExpungeAllInFolderTaskJSON,
  FolderJSON,
  LabelJSON,
  MessageJSON,
  SendDraftTaskJSON,
  SyncbackCategoryTaskJSON,
  SyncbackDraftTaskJSON,
  SyncbackMetadataTaskJSON,
  TaskBaseJSON,
} from './types';

function base(accountId: string): Omit<TaskBaseJSON, '__cls'> {
  return { id: generateId(), aid: accountId, v: 0, status: 'local' };
}

interface MailTarget {
  accountId: string;
  threadIds: string[];
  messageIds?: string[];
  source?: string;
  canBeUndone?: boolean;
}

function mail({ accountId, threadIds, messageIds = [], source, canBeUndone = true }: MailTarget) {
  return { ...base(accountId), threadIds, messageIds, source, canBeUndone, isUndo: false };
}

export function changeUnread(t: MailTarget & { unread: boolean }): ChangeUnreadTaskJSON {
  return { ...mail(t), __cls: 'ChangeUnreadTask', unread: t.unread };
}

export function changeStarred(t: MailTarget & { starred: boolean }): ChangeStarredTaskJSON {
  return { ...mail(t), __cls: 'ChangeStarredTask', starred: t.starred };
}

export function changeFolder(
  t: MailTarget & { folder: FolderJSON; sourceFolderIds?: string[] },
): ChangeFolderTaskJSON {
  return {
    ...mail(t),
    __cls: 'ChangeFolderTask',
    folder: t.folder,
    sourceFolderIds: t.sourceFolderIds,
  };
}

export function changeLabels(
  t: MailTarget & { labelsToAdd?: LabelJSON[]; labelsToRemove?: LabelJSON[] },
): ChangeLabelsTaskJSON {
  return {
    ...mail(t),
    __cls: 'ChangeLabelsTask',
    labelsToAdd: t.labelsToAdd ?? [],
    labelsToRemove: t.labelsToRemove ?? [],
  };
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

// TODO(engine): expungeAllInFolder, syncbackCategory and destroyCategory have no local
// phase in ms-tasks: nothing changes until the remote phase and the next folder sync. UI
// built on them must wait for the Folder/Label (and Message) deltas, not the `task.queue`
// result; `created` is only filled on the completed task.
export function expungeAllInFolder(folder: FolderJSON): ExpungeAllInFolderTaskJSON {
  return { ...base(folder.aid), __cls: 'ExpungeAllInFolderTask', folder };
}

export function syncbackCategory(args: {
  accountId: string;
  path: string;
  existingPath?: string;
}): SyncbackCategoryTaskJSON {
  const { accountId, ...rest } = args;
  return { ...base(accountId), __cls: 'SyncbackCategoryTask', ...rest, created: null };
}

export function destroyCategory(accountId: string, path: string): DestroyCategoryTaskJSON {
  return { ...base(accountId), __cls: 'DestroyCategoryTask', path };
}
