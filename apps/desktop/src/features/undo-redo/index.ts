import ComponentRegistry from '../../packages/component-registry';
import WorkspaceStore from '../../data/stores/workspace';
import UndoRedoToast from './toast';

export function activate() {
  if (AppEnv.isMainWindow()) {
    ComponentRegistry.register(UndoRedoToast, {
      location: WorkspaceStore.Sheet.Global.Footer,
    });
  }
}

export function deactivate() {
  if (AppEnv.isMainWindow()) {
    ComponentRegistry.unregister(UndoRedoToast);
  }
}
