/* eslint global-require: 0 */
import {
  localized,
  PreferencesUIStore,
  PreferencesUIStoreTab,
  ComponentRegistry,
  ExtensionRegistry,
} from 'mailspring-exports';
import TemplatePicker from './picker';
import TemplateStatusBar from './status-bar';
import TemplateComposerExtension from './composer-extension';

// Module state (previously on the module object via `this`).
let savedState: object = {};
let preferencesTab: PreferencesUIStoreTab | null = null;

export function activate(state = {}) {
  savedState = state;
  preferencesTab = new PreferencesUIStore.TabItem({
    tabId: 'Templates',
    displayName: localized('Templates'),
    componentClassFn: () => require('./preferences-templates').default,
    order: 14,
    group: 'mail',
    keywords: 'templates canned responses'.split(' '),
  });
  ComponentRegistry.register(TemplatePicker, { role: 'Composer:ActionButton' });
  ComponentRegistry.register(TemplateStatusBar, { role: 'Composer:Footer' });
  PreferencesUIStore.registerPreferencesTab(preferencesTab);
  ExtensionRegistry.Composer.register(TemplateComposerExtension);
}

export function deactivate() {
  ComponentRegistry.unregister(TemplatePicker);
  ComponentRegistry.unregister(TemplateStatusBar);
  if (preferencesTab) PreferencesUIStore.unregisterPreferencesTab(preferencesTab.tabId);
  ExtensionRegistry.Composer.unregister(TemplateComposerExtension);
}

export function serialize() {
  return savedState;
}
