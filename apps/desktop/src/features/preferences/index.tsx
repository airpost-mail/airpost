/* eslint global-require: 0 */
import { localized } from '../../intl/localize';
import { PreferencesUIStore } from '../../data/stores/preferences-ui';
import WorkspaceStore from '../../data/stores/workspace';
import ComponentRegistry from '../../packages/component-registry';

import PreferencesRoot from './root';
import { migrateCoreMcpConfig } from './tabs/mcp/policy';
import { replaceStaleBridgeEntryOnce } from './tabs/mcp/tern-mcp';

/** Moves the old MCP plugin's settings into mcp.json and repoints its Claude Desktop entry at
 * tern-mcp. Main window of the real app only: both write outside the app's own state. */
function upgradeMcpSetup() {
  if (AppEnv.inSpecMode() || !AppEnv.isMainWindow()) return;
  const dir = AppEnv.getConfigDirPath();
  try {
    migrateCoreMcpConfig(dir);
  } catch (err) {
    console.warn(`[MCP] Could not move core.mcp to mcp.json: ${(err as Error).message}`);
  }
  replaceStaleBridgeEntryOnce(dir).catch((err) => console.warn(`[MCP] ${err.message}`));
}

export function activate() {
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Tern Server',
      displayName: localized('Tern server'),
      componentClassFn: () => require('./tabs/server').default,
      order: 0,
      group: 'server',
      keywords: 'server web sync sign in self-hosted tracking read receipts'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'General',
      displayName: localized('General'),
      componentClassFn: () => require('./tabs/general').default,
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
      componentClassFn: () => require('./tabs/notifications').default,
      order: 2,
      group: 'app',
      keywords: 'sound badge alerts notify snooze'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Appearance',
      displayName: localized('Appearance'),
      componentClassFn: () => require('./tabs/appearance').default,
      order: 3,
      group: 'app',
      keywords: 'theme dark light layout reading pane colours scale'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Shortcuts',
      displayName: localized('Shortcuts'),
      componentClassFn: () => require('./tabs/keymaps/keymaps').default,
      order: 4,
      group: 'app',
      keywords: 'keyboard keys keybindings gmail outlook'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Accounts',
      displayName: localized('Accounts'),
      componentClassFn: () => require('./tabs/accounts/accounts').default,
      order: 10,
      group: 'mail',
      keywords: 'account add remove aliases sender name colour reconnect password'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Reading',
      displayName: localized('Reading'),
      componentClassFn: () => require('./tabs/reading').default,
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
      componentClassFn: () => require('./tabs/composing').default,
      order: 12,
      group: 'mail',
      keywords: 'send undo reply spelling grammar rich plain text attachments warnings'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Mail Rules',
      displayName: localized('Rules'),
      componentClassFn: () => require('./tabs/mail-rules').default,
      order: 15,
      group: 'mail',
      keywords: 'filter rules automation sort'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      // The id `switchPreferencesTab` takes, which plugins may already use.
      tabId: 'Agents',
      displayName: localized('MCP'),
      componentClassFn: () => require('./tabs/agents').default,
      order: 21,
      group: 'connected',
      keywords: 'mcp agents claude codex stdio http server access key'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Integrations',
      displayName: localized('Integrations'),
      componentClassFn: () => require('./tabs/integrations').default,
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
      componentClassFn: () => require('./tabs/plugins/plugins').default,
      order: 23,
      group: 'connected',
      keywords: 'plugins extensions install services'.split(' '),
    })
  );
  PreferencesUIStore.registerPreferencesTab(
    new PreferencesUIStore.TabItem({
      tabId: 'Advanced',
      displayName: localized('Advanced'),
      componentClassFn: () => require('./tabs/advanced').default,
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

// Called as a method of this module's exports.
export function serialize(this: { state?: unknown }) {
  return this.state;
}
