import { localized } from '../../intl/localize';
import { PreferencesUIStore } from '../../data/stores/preferences-ui';

// The engine adds the default signature to the drafts it builds (draft.create); the composer's
// From row renders the signature picker (./composer-dropdown) itself.

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

  PreferencesUIStore.registerPreferencesTab(this.preferencesTab);
}

export function deactivate(this: PackageModule) {
  PreferencesUIStore.unregisterPreferencesTab(this.preferencesTab);
}

export function serialize() {
  return {};
}
