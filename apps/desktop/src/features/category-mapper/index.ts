import { localized } from '../../intl/localize';
import { PreferencesUIStore, PreferencesUIStoreTab } from '../../data/stores/preferences-ui';

let preferencesTab: PreferencesUIStoreTab | null = null;

export function activate() {
  preferencesTab = new PreferencesUIStore.TabItem({
    tabId: 'Folders',
    displayName: localized('Folders'),
    componentClassFn: () => require('./preferences-category-mapper').default,
    order: 91,
    group: 'advanced',
    keywords: 'folders special sent trash archive spam drafts roles'.split(' '),
  });

  PreferencesUIStore.registerPreferencesTab(preferencesTab);
}

export function deactivate() {
  if (preferencesTab) PreferencesUIStore.unregisterPreferencesTab(preferencesTab.tabId);
  preferencesTab = null;
}
