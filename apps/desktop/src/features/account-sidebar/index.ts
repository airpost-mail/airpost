import ComponentRegistry from '../../packages/component-registry';
import WorkspaceStore from '../../data/stores/workspace';
import AccountSidebar from './account-sidebar';
import { activateMboxExportRunner, deactivateMboxExportRunner } from './mbox-export-runner';

export function activate(state) {
  ComponentRegistry.register(AccountSidebar, { location: WorkspaceStore.Location.RootSidebar });
  activateMboxExportRunner();
}

export function deactivate(state) {
  ComponentRegistry.unregister(AccountSidebar);
  deactivateMboxExportRunner();
}
