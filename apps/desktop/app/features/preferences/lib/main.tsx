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
      tabId: 'Tern Server',
      displayName: localized('Tern server'),
      componentClassFn: () => require('./tabs/preferences-server').default,
      order: 0,
      group: 'server',
      keywords: 'server web sync sign in self-hosted tracking read receipts'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'General',
      displayName: localized('General'),
      componentClassFn: () => require('./tabs/preferences-general').default,
      order: 1,
      group: 'app',
      keywords: 'startup login background menu bar tray default mail app language clock time'.split(
        ' '
      ),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Notifications',
      displayName: localized('Notifications'),
      componentClassFn: () => require('./tabs/preferences-notifications').default,
      order: 2,
      group: 'app',
      keywords: 'sound badge alerts notify snooze'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Appearance',
      displayName: localized('Appearance'),
      componentClassFn: () => require('./tabs/preferences-appearance').default,
      order: 3,
      group: 'app',
      keywords: 'theme dark light layout reading pane colours scale'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Shortcuts',
      displayName: localized('Shortcuts'),
      componentClassFn: () => require('./tabs/preferences-keymaps').default,
      order: 4,
      group: 'app',
      keywords: 'keyboard keys keybindings gmail outlook'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Accounts',
      displayName: localized('Accounts'),
      componentClassFn: () => require('./tabs/preferences-accounts').default,
      order: 10,
      group: 'mail',
      keywords: 'account add remove aliases sender name colour reconnect password'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Reading',
      displayName: localized('Reading'),
      componentClassFn: () => require('./tabs/preferences-reading').default,
      order: 11,
      group: 'mail',
      keywords: 'mark read images headers swipe conversation order gravatar important unread'.split(
        ' '
      ),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Composing',
      displayName: localized('Composing & sending'),
      componentClassFn: () => require('./tabs/preferences-composing').default,
      order: 12,
      group: 'mail',
      keywords: 'send undo reply spelling grammar rich plain text attachments warnings'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Mail Rules',
      displayName: localized('Rules'),
      componentClassFn: () => require('./tabs/preferences-mail-rules').default,
      order: 15,
      group: 'mail',
      keywords: 'filter rules automation sort'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Agents',
      displayName: localized('Agents'),
      componentClassFn: () => require('./tabs/preferences-agents').default,
      order: 21,
      group: 'connected',
      keywords: 'agents mcp claude codex cli command line terminal scripts'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Integrations',
      displayName: localized('Integrations'),
      componentClassFn: () => require('./tabs/preferences-integrations').default,
      order: 22,
      group: 'connected',
      keywords:
        'translation translate grammar languagetool deepl contact people data labs enrichment'.split(
          ' '
        ),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Plugins',
      displayName: localized('Plugins'),
      componentClassFn: () => require('./tabs/plugins/preferences-plugins').default,
      order: 23,
      group: 'connected',
      keywords: 'plugins extensions install services'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Advanced',
      displayName: localized('Advanced'),
      componentClassFn: () => require('./tabs/preferences-advanced').default,
      order: 90,
      group: 'advanced',
      keywords: 'advanced reset rebuild database cache troubleshooting'.split(' '),
    })
  );

  // Preferences runs to the top of the window, its sidebar under the window controls.
  WorkspaceStore.defineSheet(
    'Preferences',
    { hideToolbar: true },
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
