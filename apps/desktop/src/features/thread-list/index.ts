import ComponentRegistry from '../../packages/component-registry';
import WorkspaceStore from '../../data/stores/workspace';

import ThreadList from './thread-list';
import ThreadListToolbar from './toolbar';
import ThreadListVertical from './vertical';
import ThreadListEmptyFolderBar from './empty-folder-bar';
import MessageListToolbar from './message-list-toolbar';
import SelectedItemsStack from './selected-items-stack';
import * as ThreadPermalinkHandler from './permalink-handler';

import { UpButton, DownButton, MoveButtons, FlagButtons } from './toolbar-buttons';

export function activate() {
  ThreadPermalinkHandler.activate();

  ComponentRegistry.register(ThreadListEmptyFolderBar, {
    location: WorkspaceStore.Location.ThreadList,
  });

  ComponentRegistry.register(ThreadList, {
    location: WorkspaceStore.Location.ThreadList,
    role: 'ThreadList',
    modes: ['split', 'list'],
  });

  ComponentRegistry.register(ThreadListVertical, {
    location: WorkspaceStore.Location.ThreadList,
    modes: ['splitVertical'],
  });

  ComponentRegistry.register(SelectedItemsStack, {
    location: WorkspaceStore.Location.MessageList,
    modes: ['split'],
  });

  // Toolbars
  ComponentRegistry.register(ThreadListToolbar, {
    location: WorkspaceStore.Location.ThreadList.Toolbar,
    modes: ['list'],
  });

  ComponentRegistry.register(MessageListToolbar, {
    location: WorkspaceStore.Location.MessageList.Toolbar,
  });

  ComponentRegistry.register(DownButton, {
    location: WorkspaceStore.Location.MessageList.Toolbar,
    modes: ['list'],
  });

  ComponentRegistry.register(UpButton, {
    location: WorkspaceStore.Location.MessageList.Toolbar,
    modes: ['list'],
  });

  ComponentRegistry.register(MoveButtons, {
    role: 'ThreadActionsToolbarButton',
  });

  ComponentRegistry.register(FlagButtons, {
    role: 'ThreadActionsToolbarButton',
  });
}

export function deactivate() {
  ComponentRegistry.unregister(ThreadList);
  ComponentRegistry.unregister(SelectedItemsStack);
  ComponentRegistry.unregister(ThreadListToolbar);
  ComponentRegistry.unregister(MessageListToolbar);
  ComponentRegistry.unregister(MoveButtons);
  ComponentRegistry.unregister(FlagButtons);
  ComponentRegistry.unregister(UpButton);
  ComponentRegistry.unregister(DownButton);
}
