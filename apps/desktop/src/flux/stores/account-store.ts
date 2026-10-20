/* eslint global-require: 0 */

import _ from 'underscore';

import MailspringStore from 'mailspring-store';
import KeyManager from '../../key-manager';
import { revokeOAuthGrant } from '../../oauth-revocation';
import * as EngineAccounts from '../../engine-accounts';
import * as Actions from '../actions';
import { Account } from '../models/account';
import { Thread } from '../models/thread';
import { Contact } from '../models/contact';
import * as Utils from '../models/utils';
import EngineEvents, { EngineNotification } from '../../engine-events';
import { CONFIG_ACCOUNTS_TO_REMOVE_KEY } from '../../shared/config-keys';
import {
  AccountChangedParams,
  AccountListResult,
  ErrorCode,
  Feature,
  Method,
} from '../../shared/engine-protocol';

const configAccountsKey = 'accounts';
const configVersionKey = 'accountsVersion';
const configСontainerFolderDefaultKey = 'containerFolderDefault';

export type IAliasSet = Array<Contact & { isAlias?: boolean }>;

// The account fields this app keeps for itself rather than take from the engine's copy.
const LOCAL_ACCOUNT_FIELDS = ['syncState', 'syncError', 'sync_error'];

/*
Public: The AccountStore listens to changes to the available accounts in
the database and exposes the currently active Account via {::current}

The accounts come from config.json. In the main window, an engine that keeps the accounts
(Feature.Accounts) adds to them: after each hello the accounts in its account.list that
config.json lacks (another client added them: `tern account add`, the web) join the list,
and account.changed brings an account's new fields. config.json stays the copy this app
edits, so account.list doesn't overwrite an account it already has: the engine's copy is the
one last added, which may be older. Accounts deleted here stay out (accountsToRemoveFromEngine).

Section: Stores
*/
export class _AccountStore extends MailspringStore {
  private _version: number;
  private _engine: typeof EngineEvents;
  private _accounts: Account[];
  private _containerFolderDefault: string;
  private _caches: { [key: string]: any };
  // Deleted in this window, for an account.list answered before the engine removed them.
  private _removedIds = new Set<string>();

  constructor(engine: typeof EngineEvents = EngineEvents) {
    super();
    this._engine = engine;
    this._loadAccounts();
    this.listenTo(Actions.removeAccount, this._onRemoveAccount);
    this.listenTo(Actions.updateAccount, this._onUpdateAccount);
    this.listenTo(Actions.reorderAccount, this._onReorderAccount);
    this.listenTo(Actions.updateContainerFolderDefault, this._onUpdateContainerFolderDefault);
    if (AppEnv.isMainWindow()) {
      engine.on('connected', this._onEngineConnected);
      engine.on('notification', this._onEngineNotification);
      if (engine.has(Feature.Accounts)) this._onEngineConnected();
    }

    AppEnv.config.onDidChange(configVersionKey, async (change) => {
      // If we already have this version of the accounts config, it means we
      // are the ones who saved the change, and we don't need to reload.
      if (this._version / 1 === change.newValue / 1) {
        return;
      }

      const oldAccountIds = this._accounts.map((a) => a.id);
      this._loadAccounts();
      const accountIds = this._accounts.map((a) => a.id);
      const newAccountIds = _.difference(accountIds, oldAccountIds);

      if (AppEnv.isMainWindow() && newAccountIds.length > 0) {
        const newId = newAccountIds[0];
        Actions.focusDefaultMailboxPerspectiveForAccounts([newId], {
          sidebarAccountIds: accountIds,
        });
        const FolderSyncProgressStore = require('./folder-sync-progress-store').default;
        await FolderSyncProgressStore.whenCategoryListSynced(newId);
        Actions.focusDefaultMailboxPerspectiveForAccounts([newId], {
          sidebarAccountIds: accountIds,
        });
        // Uncollapse the inbox for the new account after the sidebar updates
        Actions.setCollapsedSidebarItem('Inbox', false);
      }
    });
  }

  isMyEmail(emailOrEmails: string | string[] = []) {
    const myEmails = this.emailAddresses();
    const emails = typeof emailOrEmails === 'string' ? [emailOrEmails] : emailOrEmails;

    for (const email of emails) {
      if (myEmails.find((myEmail) => Utils.emailIsEquivalent(myEmail, email))) {
        return true;
      }
    }
    return false;
  }

  _loadAccounts = () => {
    try {
      this._caches = {};
      this._version = AppEnv.config.get(configVersionKey) || 0;
      this._accounts = [];
      this._containerFolderDefault = AppEnv.config.get(configСontainerFolderDefaultKey) || '';
      for (const json of AppEnv.config.get(configAccountsKey) || []) {
        this._accounts.push(new Account({}).fromJSON(json));
      }

      // Run a few checks on account consistency. We want to display useful error
      // messages and these can result in very strange exceptions downstream otherwise.
      this._enforceAccountsValidity();
    } catch (error) {
      AppEnv.reportError(error);
    }

    this._trigger();
  };

  _onEngineConnected = () => {
    if (!this._engine.has(Feature.Accounts)) return;
    this._engine.request<AccountListResult>(Method.AccountList, {}).then(
      (result) => {
        const jsons = ((result && result.accounts) || []).map((a) => a.account);
        this._takeEngineAccounts(jsons, { update: false });
      },
      (err) => console.warn(`account.list failed; showing the accounts in config.json: ${err}`)
    );
  };

  _onEngineNotification = ({ method, params }: EngineNotification) => {
    if (method !== Method.AccountChanged || !this._engine.has(Feature.Accounts)) return;
    const { account } = (params || {}) as AccountChangedParams;
    this._takeEngineAccounts([account], { update: true });
  };

  // Adds the engine's accounts this list lacks and, with `update`, applies the engine's fields
  // to the ones it has. Saves (and so tells the supervisor) only when something changed.
  _takeEngineAccounts = (jsons: Array<Record<string, any>>, { update }: { update: boolean }) => {
    const removed = new Set<string>(AppEnv.config.get(CONFIG_ACCOUNTS_TO_REMOVE_KEY) || []);
    let changed = false;
    for (const json of jsons) {
      if (!json || !json.id || !json.emailAddress) continue;
      if (removed.has(json.id) || this._removedIds.has(json.id)) continue;
      const idx = this._accounts.findIndex((a) => a.id === json.id);
      if (idx === -1) {
        // Two accounts with one address would be dropped at the next load.
        if (this._accounts.some((a) => a.emailAddress === json.emailAddress)) continue;
        this._accounts.push(new Account({}).fromJSON(json));
        changed = true;
        continue;
      }
      if (!update) continue;
      const current = this._accounts[idx].toJSON();
      const next = { ...current, settings: { ...(current.settings || {}) } };
      let differs = false;
      for (const [key, value] of Object.entries(json)) {
        if (LOCAL_ACCOUNT_FIELDS.includes(key)) continue;
        if (key === 'settings' && value && typeof value === 'object') {
          for (const [k, v] of Object.entries(value)) {
            if (JSON.stringify(next.settings[k]) === JSON.stringify(v)) continue;
            next.settings[k] = v;
            differs = true;
          }
        } else if (JSON.stringify(next[key]) !== JSON.stringify(value)) {
          next[key] = value;
          differs = true;
        }
      }
      if (!differs) continue;
      this._accounts[idx] = new Account({}).fromJSON(next);
      changed = true;
    }
    if (!changed) return;
    this._caches = {};
    this._save();
  };

  _enforceAccountsValidity = () => {
    const seenIds = {};
    const seenEmails = {};
    let message = null;

    this._accounts = this._accounts.filter((account) => {
      if (!account.emailAddress) {
        message =
          'Assertion failure: One of the accounts in config.json did not have an emailAddress, and was removed. You should re-link the account.';
        return false;
      }
      if (seenIds[account.id]) {
        message =
          'Assertion failure: Two of the accounts in config.json had the same ID and one was removed. Please give each account a separate ID.';
        return false;
      }
      if (seenEmails[account.emailAddress]) {
        message =
          'Assertion failure: Two of the accounts in config.json had the same email address and one was removed.';
        return false;
      }

      seenIds[account.id] = true;
      seenEmails[account.emailAddress] = true;
      return true;
    });

    if (message && AppEnv.isMainWindow()) {
      AppEnv.showErrorDialog(`Tern was unable to load your account preferences.\n\n${message}`);
    }
  };

  _trigger() {
    for (const account of this._accounts) {
      if (!account || !account.id) {
        const err = new Error('An invalid account was added to `this._accounts`');
        AppEnv.reportError(err);
        this._accounts = this._accounts.filter(Boolean);
      }
    }
    this.trigger();
  }

  _save = () => {
    this._version += 1;
    const configAccounts = this._accounts.map((a) => a.toJSON());
    configAccounts.forEach((a) => {
      delete a.sync_error;

      // this should not be necessary since this info is stripped when
      // the account is added, but we want to be on the safe side.
      delete a.settings.imap_password;
      delete a.settings.smtp_password;
      delete a.settings.refresh_token;
    });
    AppEnv.config.set(configAccountsKey, configAccounts);
    AppEnv.config.set(configVersionKey, this._version);
    AppEnv.config.set(configСontainerFolderDefaultKey, this._containerFolderDefault);
    this._trigger();
  };

  /**
   * Actions.updateAccount is called directly from the local-sync worker.
   * This will update the account with its updated sync state
   */
  _onUpdateAccount = (id: string, updated: Partial<Account>) => {
    const idx = this._accounts.findIndex((a) => a.id === id);
    let account = this._accounts[idx];
    if (!account) return;
    account = Object.assign(account, updated);
    this._caches = {};
    this._accounts[idx] = account;
    this._save();
    this._updateInEngine(account, updated);
  };

  // The registry keeps what the user edits (name, aliases, settings); the engine answers with
  // account.changed. The sync state is this app's own.
  _updateInEngine(account: Account, updated: Partial<Account>) {
    if (!EngineAccounts.accountSetupAvailable()) return;
    const json = account.toJSON();
    const patch: Record<string, any> = {};
    for (const key of Object.keys(updated)) {
      if (LOCAL_ACCOUNT_FIELDS.includes(key) || key === 'id' || !(key in json)) continue;
      patch[key] = json[key];
    }
    if (patch.settings) {
      patch.settings = { ...patch.settings };
      for (const key of EngineAccounts.SECRET_SETTINGS) delete patch.settings[key];
    }
    if (!Object.keys(patch).length) return;
    EngineAccounts.updateAccount(account.id, patch).catch((err) =>
      console.warn(`account.update ${account.id} failed: ${err && err.message}`)
    );
  }

  /**
   * When an account is removed from Mailspring, the AccountStore
   * triggers. The local-sync/src/local-sync-worker/index.js listens to
   * the AccountStore and runs `ensureK2Consistency`. This will actually
   * delete the Account on the local sync side.
   */
  _onRemoveAccount = async (id: string) => {
    const account = this._accounts.find((a) => a.id === id);
    if (!account) return;

    this._caches = {};
    this._removedIds.add(id);

    // An engine with account setup stops the account, revokes its Google grant and deletes
    // its rows and secrets itself (account.remove {revoke, deleteData}); that runs before
    // config.json lets go of the account, since the supervisor's own account.remove would
    // delete the token the revocation needs.
    const removedByEngine = await this._removeFromEngine(account);
    // Otherwise started before anything below clears the keychain entry it needs to read.
    const revocation = removedByEngine ? Promise.resolve() : revokeOAuthGrant(account);

    const remainingAccounts = this._accounts.filter((a) => a !== account);
    // This action is called before saving because we need to unfocus the
    // perspective of the account that is being removed before removing the
    // account, otherwise when we trigger with the new set of accounts, the
    // current perspective will still reference a stale accountId which will
    // cause things to break
    Actions.focusDefaultMailboxPerspectiveForAccounts(remainingAccounts);
    _.defer(() => {
      Actions.setCollapsedSidebarItem('Inbox', true);
    });

    this._accounts = remainingAccounts;
    this._save();

    if (remainingAccounts.length === 0) {
      // Clear everything and logout. application:reset-database destroys this window
      // before relaunching, which would cancel an in-flight revocation request. It only
      // deletes the database, so the secrets in config.json must be removed here.
      await revocation;
      await KeyManager.deleteAccountSecrets(account);
      const ipc = require('electron').ipcRenderer;
      ipc.send('command', 'application:reset-database', { keepMetadata: false });
    } else if (removedByEngine) {
      KeyManager.deleteAccountSecrets(account);
    } else {
      // Clear the cached data for the account and reset secrets once that has completed
      AppEnv.mailsyncBridge
        .resetCacheForAccount(account, { silent: true, keepMetadata: false })
        .then(() => {
          console.log('Account removal complete.');
          KeyManager.deleteAccountSecrets(account);
        });
    }
  };

  async _removeFromEngine(account: Account) {
    if (!EngineAccounts.accountSetupAvailable()) return false;
    try {
      await EngineAccounts.removeAccount(account.id);
      return true;
    } catch (err) {
      if (err && err.code === ErrorCode.UnknownAccount) return false;
      console.warn(`The engine could not remove account ${account.id}: ${err && err.message}`);
      return false;
    }
  }

  _onReorderAccount = (id: string, newIdx: number) => {
    const existingIdx = this._accounts.findIndex((a) => a.id === id);
    if (existingIdx === -1) return;
    const account = this._accounts[existingIdx];
    this._caches = {};
    this._accounts.splice(existingIdx, 1);
    this._accounts.splice(newIdx, 0, account);
    this._save();
    if (EngineAccounts.accountSetupAvailable()) {
      EngineAccounts.reorderAccounts(this.accountIds()).catch((err) =>
        console.warn(`account.reorder failed: ${err && err.message}`)
      );
    }
  };

  _onUpdateContainerFolderDefault = (containerFolderDefault: string) => {
    this._containerFolderDefault = containerFolderDefault;
    AppEnv.config.set(configСontainerFolderDefaultKey, this._containerFolderDefault);
  };

  addAccount = async (account: Account) => {
    if (!account.emailAddress || !account.provider || !(account instanceof Account)) {
      throw new Error(`Returned account data is invalid: ${JSON.stringify(account)}`);
    }

    // send the account JSON and cloud token to the KeyManager,
    // which gives us back a version with no secrets.
    const cleanAccount = await KeyManager.extractAndStoreAccountSecrets(account);
    this._removedIds.delete(cleanAccount.id);

    this._loadAccounts();

    const existingIdx = this._accounts.findIndex(
      (a) => a.id === cleanAccount.id || a.emailAddress === cleanAccount.emailAddress
    );

    if (existingIdx === -1) {
      this._accounts.push(cleanAccount);
    } else {
      const existing = this._accounts[existingIdx];
      existing.syncState = Account.SYNC_STATE_OK;
      existing.name = cleanAccount.name;
      existing.authedAt = cleanAccount.authedAt;
      existing.emailAddress = cleanAccount.emailAddress;
      existing.settings = cleanAccount.settings;
    }

    this._save();
  };

  _cachedGetter<T>(key: string, fn: () => T) {
    this._caches[key] = this._caches[key] || fn();
    return this._caches[key] as T;
  }

  containerFolderDefaultGetter = () => {
    return this._containerFolderDefault;
  };

  // Public: Returns an {Array} of {Account} objects
  accounts = () => {
    return this._accounts;
  };

  accountIds = () => {
    return this._accounts.map((a) => a.id);
  };

  accountsForItems = (items: Thread[]) => {
    const accounts: { [id: string]: Account } = {};
    items.filter(Boolean).forEach(({ accountId }) => {
      accounts[accountId] = accounts[accountId] || this.accountForId(accountId);
    });
    return Object.values(accounts).filter(Boolean);
  };

  accountForItems = (items: Thread[]) => {
    const accounts = this.accountsForItems(items);
    if (accounts.length > 1) {
      return null;
    }
    return accounts[0];
  };

  // Public: Returns the {Account} for the given email address, or null.
  accountForEmail = (email: string) => {
    for (const account of this.accounts()) {
      if (Utils.emailIsEquivalent(email, account.emailAddress)) {
        return account;
      }
    }
    for (const alias of this.aliases()) {
      if (Utils.emailIsEquivalent(email, alias.email)) {
        return this.accountForId(alias.accountId);
      }
    }
    return null;
  };

  // Public: Returns the {Account} for the given account id, or null.
  accountForId(id: string) {
    return this._cachedGetter(`accountForId:${id}`, () => this._accounts.find((a) => a.id === id));
  }

  emailAddresses() {
    let addresses = (this.accounts() ? this.accounts() : []).map((a) => a.emailAddress);
    addresses = addresses.concat((this.aliases() ? this.aliases() : []).map((a) => a.email));
    return [...new Set(addresses)];
  }

  aliases(): IAliasSet {
    return this._cachedGetter<IAliasSet>('aliases', () => {
      const aliases = [];
      for (const acc of this._accounts) {
        aliases.push(acc.me());
        for (const alias of acc.aliases) {
          const aliasContact = acc.meUsingAlias(alias);
          aliasContact.isAlias = true;
          aliases.push(aliasContact);
        }
      }
      return aliases as IAliasSet;
    });
  }

  aliasesFor(accountsOrIds: Array<Account | string>): IAliasSet {
    const ids = accountsOrIds.map((accOrId) => {
      return accOrId instanceof Account ? accOrId.id : accOrId;
    });
    return this.aliases().filter((contact) => ids.includes(contact.accountId));
  }

  // Public: Returns the currently active {Account}.
  current() {
    throw new Error('AccountStore.current() has been deprecated.');
  }
}

export const AccountStore = new _AccountStore();
