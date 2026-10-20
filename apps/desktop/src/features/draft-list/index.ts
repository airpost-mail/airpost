import { WorkspaceStore, ComponentRegistry, Actions } from 'mailspring-exports';
import DraftList from './draft-list';
import DraftListToolbar from './toolbar';
import DraftListSendStatus from './send-status';
import DraftPreview from './preview';

export function activate() {
  WorkspaceStore.defineSheet(
    'Drafts',
    { root: true },
    {
      list: ['RootSidebar', 'DraftList'],
      split: ['RootSidebar', 'DraftList', 'DraftPreview'],
    }
  );
  WorkspaceStore.defineSheet('Draft', {}, { list: ['DraftPreview'] });

  if (
    AppEnv.savedState.perspective &&
    AppEnv.savedState.perspective.type === 'DraftsMailboxPerspective'
  ) {
    Actions.selectRootSheet(WorkspaceStore.Sheet.Drafts);
  }

  ComponentRegistry.register(DraftList, { location: WorkspaceStore.Location.DraftList });
  ComponentRegistry.register(DraftListToolbar, {
    location: WorkspaceStore.Location.DraftList.Toolbar,
  });
  ComponentRegistry.register(DraftPreview, { location: WorkspaceStore.Location.DraftPreview });
  ComponentRegistry.register(DraftListSendStatus, { role: 'DraftList:DraftStatus' });
}

export function deactivate() {
  ComponentRegistry.unregister(DraftList);
  ComponentRegistry.unregister(DraftListToolbar);
  ComponentRegistry.unregister(DraftPreview);
  ComponentRegistry.unregister(DraftListSendStatus);
}
