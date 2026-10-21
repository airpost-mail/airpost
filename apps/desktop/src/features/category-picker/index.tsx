import ToolbarCategoryPicker from './toolbar-category-picker';
import ComponentRegistry from '../../packages/component-registry';

export function activate() {
  ComponentRegistry.register(ToolbarCategoryPicker, { role: 'ThreadActionsToolbarButton' });
}

export function deactivate() {
  ComponentRegistry.unregister(ToolbarCategoryPicker);
}
