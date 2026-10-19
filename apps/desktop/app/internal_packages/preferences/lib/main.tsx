/* eslint global-require: 0 */
import {
  localized,
  PreferencesUIStore,
  WorkspaceStore,
  ComponentRegistry,
} from 'mailspring-exports';

import PreferencesRoot from './preferences-root';
import { migrateCoreMcpConfig } from './tabs/mcp/mcp-policy';
import { replaceStaleBridgeEntryOnce } from './tabs/mcp/tern-mcp';

/** Moves the old MCP plugin's settings into mcp.json and repoints its Claude Desktop entry at
 * tern-mcp. Main window of the real app only: both write outside the app's own state. */
function upgradeMcpSetup() {
  if (AppEnv.inSpecMode() || !AppEnv.isMainWindow()) return;
  const dir = AppEnv.getConfigDirPath();
  try {
    migrateCoreMcpConfig(dir);
  } catch (err) {
    console.warn(`[MCP] Could not move core.mcp to mcp.json: ${err.message}`);
  }
  replaceStaleBridgeEntryOnce(dir).catch((err) => console.warn(`[MCP] ${err.message}`));
}

export function activate() {
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'General',
      displayName: localized('General'),
      componentClassFn: () => require('./tabs/preferences-general').default,
      order: 1,
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Accounts',
      displayName: localized('Accounts'),
      componentClassFn: () => require('./tabs/preferences-accounts').default,
      order: 2,
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Integrations',
      displayName: localized('Integrations'),
      componentClassFn: () => require('./tabs/preferences-integrations').default,
      order: 3,
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Appearance',
      displayName: localized('Appearance'),
      componentClassFn: () => require('./tabs/preferences-appearance').default,
      order: 4,
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Shortcuts',
      displayName: localized('Shortcuts'),
      componentClassFn: () => require('./tabs/preferences-keymaps').default,
      order: 5,
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Mail Rules',
      displayName: localized('Mail Rules'),
      componentClassFn: () => require('./tabs/preferences-mail-rules').default,
      order: 6,
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Plugins',
      displayName: localized('Plugins'),
      componentClassFn: () => require('./tabs/plugins/preferences-plugins').default,
      order: 7,
    })
  );

  WorkspaceStore.defineSheet(
    'Preferences',
    {},
    {
      list: ['Preferences'],
      split: ['Preferences'],
      splitVertical: ['Preferences'],
    }
  );

  ComponentRegistry.register(PreferencesRoot, {
    location: WorkspaceStore.Location.Preferences,
  });

  setTimeout(upgradeMcpSetup, 5000);
}

export function deactivate() {}

export function serialize() {
  return this.state;
}
