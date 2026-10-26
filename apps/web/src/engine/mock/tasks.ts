// The thread and category task builders the mock engine plans its intents with, as the
// engine's tern-tasks does (crates/tern-tasks/src/intents.rs); the web itself calls the
// intents.
import { base, mail, type MailTarget } from '../tasks';
import type {
  ChangeFolderTaskJSON,
  ChangeLabelsTaskJSON,
  ChangeStarredTaskJSON,
  DestroyCategoryTaskJSON,
  ExpungeAllInFolderTaskJSON,
  FolderJSON,
  LabelJSON,
  SyncbackCategoryTaskJSON,
} from '../types';

export { changeUnread, syncbackMetadata, sendDraft, syncbackDraft, destroyDraft } from '../tasks';

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

// expungeAllInFolder, syncbackCategory and destroyCategory have no local
// phase in tern-tasks: nothing changes until the remote phase and the next folder sync. UI
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
