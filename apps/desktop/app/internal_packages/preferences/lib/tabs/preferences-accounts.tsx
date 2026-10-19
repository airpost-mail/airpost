import React from 'react';
import { ipcRenderer } from 'electron';
import {
  AccountStore,
  Actions,
  Account,
  FolderSyncProgressStore,
  OnlineStatusStore,
  localized,
} from 'mailspring-exports';
import { SettingsPage, SettingsSection } from 'mailspring-component-kit';
import PreferencesAccountList from './preferences-account-list';
import PreferencesAccountDetails from './preferences-account-details';

interface PreferencesAccountsState {
  accounts: Account[];
  selected: Account;
  offlineAccountIds: string[];
  syncingAccountIds: string[];
}

class PreferencesAccounts extends React.Component<
  Record<string, unknown>,
  PreferencesAccountsState
> {
  static displayName = 'PreferencesAccounts';

  unsubscribers: (() => void)[] = [];

  constructor(props) {
    super(props);
    this.state = this.getStateFromStores();
  }

  componentDidMount() {
    this.unsubscribers = [
      AccountStore.listen(this._onAccountsChanged),
      OnlineStatusStore.listen(this._onAccountsChanged),
      FolderSyncProgressStore.listen(this._onAccountsChanged),
    ];
  }

  componentWillUnmount() {
    this.unsubscribers.forEach((unsubscribe) => unsubscribe());
  }

  getStateFromStores({ selected }: { selected?: Account } = {}) {
    const accounts = AccountStore.accounts();
    let selectedAccount;
    if (selected) {
      selectedAccount = accounts.find((a) => a.id === selected.id);
    }
    // If selected was null or no longer exists in the AccountStore,
    // just use the first account.
    if (!selectedAccount) {
      selectedAccount = accounts[0];
    }
    return {
      accounts,
      selected: selectedAccount,
      offlineAccountIds: OnlineStatusStore.offlineAccountIds(),
      syncingAccountIds: accounts
        .filter((a) => FolderSyncProgressStore.isSyncingAccount(a.id))
        .map((a) => a.id),
    };
  }

  _onAccountsChanged = () => {
    this.setState(this.getStateFromStores(this.state));
  };

  _onAddAccount() {
    ipcRenderer.send('command', 'application:add-account');
  }

  _onReorderAccount(account: Account, oldIdx: number, newIdx: number) {
    Actions.reorderAccount(account.id, newIdx);
  }

  _onSelectAccount = (account: Account) => {
    this.setState({ selected: account });
  };

  _onRemoveAccount(account: Account) {
    Actions.removeAccount(account.id);
  }

  _onAccountUpdated(account: Account, updates: Partial<Account>) {
    Actions.updateAccount(account.id, updates);
  }

  render() {
    const { accounts, selected, offlineAccountIds, syncingAccountIds } = this.state;
    return (
      <SettingsPage
        className="preferences-accounts"
        title={localized('Accounts')}
        description={localized('Mail accounts on this computer.')}
      >
        <SettingsSection
          bare
          note={
            <>
              <a onClick={this._onAddAccount}>{localized('Add account…')}</a>
              {accounts.length > 1 && (
                <span className="account-list-hint">
                  {localized('Drag an account to change its place in the sidebar.')}
                </span>
              )}
            </>
          }
        >
          <PreferencesAccountList
            accounts={accounts}
            selected={selected}
            offlineAccountIds={offlineAccountIds}
            syncingAccountIds={syncingAccountIds}
            onReorderAccount={this._onReorderAccount}
            onSelectAccount={this._onSelectAccount}
          />
        </SettingsSection>
        {selected && (
          <PreferencesAccountDetails
            key={selected.id}
            account={selected}
            isOffline={offlineAccountIds.includes(selected.id)}
            onAccountUpdated={this._onAccountUpdated}
            onRemoveAccount={this._onRemoveAccount}
          />
        )}
      </SettingsPage>
    );
  }
}

export default PreferencesAccounts;
