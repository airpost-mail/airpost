import {
  localized,
  PreferencesUIStore,
  ExtensionRegistry,
  ComponentRegistry,
} from 'mailspring-exports';

import SignatureComposerExtension from './composer-extension';
import SignatureComposerDropdown from './composer-dropdown';

export function activate() {
  this.preferencesTab = new PreferencesUIStore.TabItem({
    tabId: 'Signatures',
    displayName: localized('Signatures'),
    componentClassFn: () => require('./preferences-signatures').default, // eslint-disable-line
    order: 13,
    group: 'mail',
    keywords: 'signature'.split(' '),
  });

  ExtensionRegistry.Composer.register(SignatureComposerExtension);
  PreferencesUIStore.registerPreferencesTab(this.preferencesTab);

  ComponentRegistry.register(SignatureComposerDropdown, {
    role: 'Composer:FromFieldComponents',
  });
}

export function deactivate() {
  ExtensionRegistry.Composer.unregister(SignatureComposerExtension);
  PreferencesUIStore.unregisterPreferencesTab(this.preferencesTab.sectionId);

  ComponentRegistry.unregister(SignatureComposerDropdown);
}

export function serialize() {
  return {};
}
