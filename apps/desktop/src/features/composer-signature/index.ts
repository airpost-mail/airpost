import { localized } from '../../intl/localize';
import { PreferencesUIStore } from '../../data/stores/preferences-ui';
import * as ExtensionRegistry from '../../packages/extension-registry';

import SignatureComposerExtension from './composer-extension';

// The composer's From row renders the signature picker (./composer-dropdown) itself.

type PackageModule = { preferencesTab: InstanceType<typeof PreferencesUIStore.TabItem> };

// activate and deactivate are called as methods of this module's exports.
export function activate(this: PackageModule) {
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
}

export function deactivate(this: PackageModule) {
  ExtensionRegistry.Composer.unregister(SignatureComposerExtension);
  PreferencesUIStore.unregisterPreferencesTab(this.preferencesTab);
}

export function serialize() {
  return {};
}
