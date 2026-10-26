import { imapUtf7 } from '../../mail/imap-utf7';

import fs from 'fs';
import _str from 'underscore.string';
import React from 'react';
import OutlineViewItem from '../../components/outline-view-item';
import { LabelColorizer } from '../../components/mail-label';
import LabelColorPopover from '../../components/label-color-popover';
import { MailboxPerspective } from '../../data/mailbox-perspective';
import { Label } from '../../data/models/label';
import { Folder } from '../../data/models/folder';
import { Category } from '../../data/models/category';
import LabelColorStore from '../../data/stores/label-color';
import FocusedPerspectiveStore from '../../data/stores/focused-perspective';
import { IntentTask } from '../../data/tasks/intent';
import { Method } from '../../engine/protocol';
import { GetManyRFC2822Task } from '../../data/tasks/get-many-rfc2822';
import CategoryStore from '../../data/stores/category';
import * as Actions from '../../data/actions';
import RegExpUtils from '../../utils/regexp';
import * as DragDropTypes from '../../components/drag-drop-types';
import { localized } from '../../intl/localize';
import { Dialogs } from '../../platform/app-api';
import TaskQueue from '../../data/stores/task-queue';

import * as SidebarActions from './sidebar-actions';
import { ISidebarItem } from './types';

const idForCategories = (categories: { id: string }[]) => categories.map((c) => c.id).join('-');

const countForItem = function (perspective: MailboxPerspective) {
  const unreadCountEnabled = AppEnv.config.get('core.workspace.showUnreadForAllCategories');
  if (perspective.isInbox() || unreadCountEnabled) {
    return perspective.unreadCount();
  }
  return 0;
};

const isItemSelected = (perspective: MailboxPerspective) =>
  FocusedPerspectiveStore.current().isEqual(perspective);

const isItemCollapsed = function (id: string) {
  if (AppEnv.savedState.sidebarKeysCollapsed[id] !== undefined) {
    return AppEnv.savedState.sidebarKeysCollapsed[id];
  } else {
    return true;
  }
};

const toggleItemCollapsed = function (item: ISidebarItem) {
  if (!(item.children.length > 0)) {
    return;
  }
  SidebarActions.setKeyCollapsed(item.id, !isItemCollapsed(item.id));
};

// Categories whose delete confirmation is open. The window takes input meanwhile, so a second
// Delete on the same folder is ignored rather than queueing a second task.
const confirmingDelete = new Set<string>();

const onDeleteItem = async function (item: ISidebarItem) {
  if (item.deleted === true) {
    return;
  }
  const category = item.perspective.category();
  if (!category) {
    return;
  }
  if (confirmingDelete.has(category.id)) {
    return;
  }

  confirmingDelete.add(category.id);
  let response: number;
  try {
    response = await Dialogs.ask(
      {
        type: 'info',
        message: localized('Are you sure?'),
        detail: localized(
          'Deleting folders and labels cannot be undone and it may take a few minutes for changes to sync to Tern.'
        ),
        buttons: [localized('Delete'), localized('Cancel')],
        defaultId: 0,
      },
      'none'
    );
  } finally {
    confirmingDelete.delete(category.id);
  }

  if (response !== 0) {
    return;
  }

  Actions.queueTask(categoryIntent(Method.CategoriesDelete, category));
};

// A `categories.*` intent on one folder or label.
function categoryIntent(intent: string, category: Category, params: Record<string, unknown> = {}) {
  return new IntentTask({
    intent,
    params: { categoryId: category.id, ...params },
    accountId: category.accountId,
  });
}

const EXCLUDED_EXPORT_ROLES = new Set(['drafts', 'starred', 'unread']);

const onExportFolder = function (item: ISidebarItem) {
  const category = item.perspective.category();
  if (!category) {
    return;
  }

  AppEnv.showOpenDialog(
    {
      title: localized('Export folder as .eml files'),
      buttonLabel: localized('Export'),
      properties: ['openDirectory', 'createDirectory'],
    },
    (selected: string[]) => {
      if (!selected || selected.length === 0) {
        return;
      }
      const outputDir = selected[0];
      Actions.queueTask(
        new GetManyRFC2822Task({
          accountId: category.accountId,
          folderId: category.id,
          folderPath: category.path,
          outputDir,
        })
      );
    }
  );
};

const onExportMboxFolder = function (item: ISidebarItem) {
  const category = item.perspective.category();
  if (!category) {
    return;
  }

  const defaultName = `${(category.displayName || 'folder').replace(/[/?<>\\:*|"]/g, '_')}.mbox`;

  AppEnv.showSaveDialog(
    {
      title: localized('Export folder as .mbox file'),
      buttonLabel: localized('Export'),
      defaultPath: defaultName,
      filters: [{ name: 'mbox', extensions: ['mbox'] }],
    },
    (mboxPath: string) => {
      if (!mboxPath) {
        return;
      }

      // Two exports feeding one destination would fight over the staging
      // directory and interleave appends into the same file.
      const running = TaskQueue.findTasks(GetManyRFC2822Task, (t: GetManyRFC2822Task) => {
        return t.format === 'mbox' && t.mboxPath === mboxPath;
      });
      if (running.length > 0) {
        AppEnv.showErrorDialog(localized('An mbox export to this file is already in progress.'));
        return;
      }

      // The sync engine writes one .eml file per message into a staging
      // directory beside the destination file (same volume as the export the
      // user sized, and discoverable if something goes wrong); the mbox
      // export runner incrementally assembles it into the mbox as the export
      // progresses — including across app restarts. The staging path carries a
      // per-export token so a new export never shares a directory (or working
      // file) with an earlier, still-lingering completed task for the same
      // destination — that collision would let the old task finalize against
      // the new export's staging and overwrite the destination.
      const token = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
      const stagingDir = `${mboxPath}.${token}.partial`;
      fs.mkdirSync(stagingDir, { recursive: true });

      Actions.queueTask(
        new GetManyRFC2822Task({
          accountId: category.accountId,
          folderId: category.id,
          folderPath: category.path,
          outputDir: stagingDir,
          format: 'mbox',
          mboxPath,
        })
      );
    }
  );
};

const onMarkAllAsRead = function (item: ISidebarItem) {
  const category = item.perspective.category();
  if (!category) {
    return;
  }

  // The engine finds the unread threads (in all mail, unless it is Trash or Spam).
  Actions.queueTask(categoryIntent(Method.CategoriesMarkRead, category));
};

function detectFolderSeparator(accountId: string): string {
  // Check category paths for known prefixes — most reliable signal
  for (const cat of CategoryStore.categories(accountId)) {
    const catPath = cat.path;
    for (const prefix of ['INBOX', '[Gmail]', 'Tern', '[Mailspring]', 'Mailspring']) {
      if (catPath.startsWith(prefix) && catPath.length > prefix.length) {
        const ch = catPath[prefix.length];
        if (ch === '.' || ch === '/' || ch === '\\') return ch;
      }
    }
  }

  return '/';
}

export function createCategory(accountId: string, name: string, parentCategory?: { path: string }) {
  if (!name) {
    return;
  }

  let fullName: string;
  if (parentCategory) {
    const separator = detectFolderSeparator(accountId);
    const decodedPath = imapUtf7.decode(parentCategory.path);
    fullName = decodedPath + separator + name;
  } else {
    fullName = name;
  }

  Actions.queueTask(
    new IntentTask({
      intent: Method.CategoriesCreate,
      params: { accountId, name: fullName },
      accountId,
    })
  );
}

const onCreateChild = function (item: ISidebarItem, childName: string) {
  const category = item.perspective.category();
  if (!category) {
    return;
  }
  createCategory(category.accountId, childName, category);
};

const onEditItem = function (item: ISidebarItem, value: string) {
  let newDisplayName;
  if (!value) {
    return;
  }
  if (item.deleted === true) {
    return;
  }
  const category = item.perspective.category();
  if (!category) {
    return;
  }
  const re = RegExpUtils.subcategorySplitRegex();
  let match = re.exec(category.displayName);
  let lastMatch = match;
  while (match) {
    lastMatch = match;
    match = re.exec(category.displayName);
  }
  if (lastMatch) {
    newDisplayName = category.displayName.slice(0, lastMatch.index + 1) + value;
  } else {
    newDisplayName = value;
  }
  if (newDisplayName === category.displayName) {
    return;
  }

  Actions.queueTask(categoryIntent(Method.CategoriesRename, category, { name: newDisplayName }));
  LabelColorStore.renameCustomColor(category.displayName, newDisplayName);
};

const onEditColor = function (item: ISidebarItem, originRect: DOMRect) {
  const category = item.perspective.category();
  if (!category) {
    return;
  }
  Actions.openPopover(React.createElement(LabelColorPopover, { label: category }), {
    originRect,
    direction: 'right',
  });
};

export default class SidebarItem {
  static forPerspective(
    id: string,
    perspective: MailboxPerspective,
    opts: Partial<ISidebarItem> = {}
  ): ISidebarItem {
    let counterStyle: string | undefined;
    if (perspective.isInbox()) {
      counterStyle = OutlineViewItem.CounterStyles.Alt;
    }

    const collapsed = isItemCollapsed(id);

    return Object.assign(
      {
        id,
        name: perspective.name,
        contextMenuLabel: perspective.name,
        count: countForItem(perspective),
        iconName: perspective.iconName,
        children: [],
        perspective,
        selected: isItemSelected(perspective),
        collapsed: collapsed != null ? collapsed : true,
        counterStyle,
        onDelete: opts.deletable ? onDeleteItem : undefined,
        onEdited: opts.editable ? onEditItem : undefined,
        onExport: opts.exportable ? onExportFolder : undefined,
        onExportMbox: opts.exportable ? onExportMboxFolder : undefined,
        onCreateChild: opts.editable ? onCreateChild : undefined,
        onCollapseToggled: toggleItemCollapsed,

        onDrop(item: ISidebarItem, event: React.DragEvent) {
          const jsonString = event.dataTransfer.getData(DragDropTypes.ThreadsDragType);
          let jsonData = null;
          try {
            jsonData = JSON.parse(jsonString);
          } catch (err) {
            console.error(`JSON parse error: ${err}`);
          }
          if (!jsonData) {
            return;
          }
          item.perspective.receiveThreadIds(jsonData.threadIds);
        },

        shouldAcceptDrop(item: ISidebarItem, event: React.DragEvent) {
          const target = item.perspective;
          const current = FocusedPerspectiveStore.current();
          if (!event.dataTransfer.types.includes(DragDropTypes.ThreadsDragType)) {
            return false;
          }
          if (target.isEqual(current)) {
            return false;
          }

          // We can't inspect the drag payload until drop, so we use a dataTransfer
          // type to encode the account IDs of threads currently being dragged.
          const accountIds = DragDropTypes.accountIdsForDragTypes(event.dataTransfer.types);
          return target.canReceiveThreadsFromAccountIds(accountIds);
        },

        onSelect(item: ISidebarItem) {
          Actions.focusMailboxPerspective(item.perspective);
        },
      },
      opts
    );
  }

  static forCategories(categories: Category[] = [], opts: Partial<ISidebarItem> = {}) {
    const id = idForCategories(categories);
    const contextMenuLabel = _str.capitalize(
      categories[0] != null ? categories[0].displayType() : ''
    );
    const perspective = MailboxPerspective.forCategories(categories);

    if (opts.deletable == null) {
      opts.deletable = true;
    }
    if (opts.editable == null) {
      opts.editable = true;
    }
    if (opts.exportable == null) {
      const role = categories[0] != null ? categories[0].role : null;
      opts.exportable = !role || !EXCLUDED_EXPORT_ROLES.has(role);
    }
    if (opts.onMarkAllAsRead == null && perspective.category()) {
      opts.onMarkAllAsRead = onMarkAllAsRead;
    }
    const category = categories.length === 1 ? categories[0] : null;
    if (category instanceof Label && !category.role) {
      opts.iconColor = LabelColorizer.backgroundColorDark(category);
      opts.onEditColor = onEditColor;
    }
    opts.contextMenuLabel = contextMenuLabel;
    return this.forPerspective(id, perspective, opts);
  }

  static forStarred(accountIds: string[], opts: Partial<ISidebarItem> = {}) {
    const perspective = MailboxPerspective.forStarred(accountIds);
    let id = 'Starred';
    if (opts.name) {
      id += `-${opts.name}`;
    }
    return this.forPerspective(id, perspective, opts);
  }

  static forUnread(accountIds: string[], opts: Partial<ISidebarItem> = {}) {
    // NOTE: It's possible for an account to not yet have an `inbox`
    // category. Since the `SidebarStore` triggers on `AccountStore`
    // changes, it'll trigger the exact moment an account is added to the
    // config. However, the API has not yet come back with the list of
    // `categories` for that account.
    const categories = accountIds
      .map((accId) => CategoryStore.getCategoryByRole(accId, 'inbox'))
      .filter((c): c is Folder | Label => !!c);

    const perspective = MailboxPerspective.forUnread(categories);
    let id = 'Unread';
    if (opts.name) {
      id += `-${opts.name}`;
    }
    return this.forPerspective(id, perspective, opts);
  }

  static forDrafts(accountIds: string[], opts: Partial<ISidebarItem> = {}) {
    const perspective = MailboxPerspective.forDrafts(accountIds);
    const id = `Drafts-${opts.name}`;
    return this.forPerspective(id, perspective, opts);
  }
}
