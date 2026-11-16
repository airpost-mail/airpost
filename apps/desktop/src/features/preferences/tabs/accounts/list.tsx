import React from 'react';
import classnames from 'classnames';
import { localized } from '../../../../intl/localize';
import { Account } from '../../../../data/models/account';
import { SettingsCard, StatusPill } from '../../../../components/settings';

interface PreferencesAccountListProps {
  accounts: Account[];
  selected: Account;
  offlineAccountIds: string[];
  syncingAccountIds: string[];
  onReorderAccount: (account: Account, oldIndex: number, newIndex: number) => void;
  onSelectAccount: (account: Account) => void;
}

interface PreferencesAccountListState {
  dragIndex: number | null;
  dropIndex: number | null;
}

export function AccountAvatar({ account }: { account: Account }) {
  const initial = (account.label || account.emailAddress || '?').trim().charAt(0).toUpperCase();
  return (
    <div
      className={classnames('account-avatar', { 'no-color': !account.color })}
      style={account.color ? { background: account.color } : undefined}
      aria-hidden
    >
      {initial}
    </div>
  );
}

export function AccountStatus({
  account,
  offline,
  syncing,
}: {
  account: Account;
  offline: boolean;
  syncing: boolean;
}) {
  if (account.syncState === Account.SYNC_STATE_AUTH_FAILED) {
    return <StatusPill kind="bad">{localized("Can't sign in")}</StatusPill>;
  }
  if (account.hasSyncStateError()) {
    return <StatusPill kind="bad">{localized('Sync error')}</StatusPill>;
  }
  if (offline) {
    return <StatusPill kind="warn">{localized('Offline, retrying')}</StatusPill>;
  }
  if (syncing) {
    return <StatusPill kind="off">{localized('Syncing')}</StatusPill>;
  }
  return <StatusPill kind="ok">{localized('Up to date')}</StatusPill>;
}

/** The accounts as cards; drag one to change its place in the sidebar. */
class PreferencesAccountList extends React.Component<
  PreferencesAccountListProps,
  PreferencesAccountListState
> {
  static displayName = 'PreferencesAccountList';

  state: PreferencesAccountListState = { dragIndex: null, dropIndex: null };

  _onDragStart = (e: React.DragEvent, idx: number) => {
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('airpost-account-index', String(idx));
    this.setState({ dragIndex: idx });
  };

  _onDragOver = (e: React.DragEvent, idx: number) => {
    if (this.state.dragIndex === null) return;
    e.preventDefault();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const dropIndex = e.clientY > rect.top + rect.height / 2 ? idx + 1 : idx;
    if (dropIndex !== this.state.dropIndex) this.setState({ dropIndex });
  };

  _onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    const { dragIndex, dropIndex } = this.state;
    this.setState({ dragIndex: null, dropIndex: null });
    if (dragIndex === null || dropIndex === null) return;
    // AccountStore takes the index after the account has been taken out of the list.
    const newIndex = dropIndex > dragIndex ? dropIndex - 1 : dropIndex;
    if (newIndex !== dragIndex) {
      this.props.onReorderAccount(this.props.accounts[dragIndex], dragIndex, newIndex);
    }
  };

  _onDragEnd = () => {
    this.setState({ dragIndex: null, dropIndex: null });
  };

  _onKeyDown = (e: React.KeyboardEvent, idx: number) => {
    const { accounts } = this.props;
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      this.props.onSelectAccount(accounts[idx]);
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const next = accounts[idx + (e.key === 'ArrowDown' ? 1 : -1)];
      if (next) {
        this.props.onSelectAccount(next);
        const sibling =
          e.key === 'ArrowDown'
            ? (e.currentTarget.nextElementSibling as HTMLElement)
            : (e.currentTarget.previousElementSibling as HTMLElement);
        sibling?.focus();
      }
    }
  };

  render() {
    const { accounts, selected, offlineAccountIds, syncingAccountIds } = this.props;
    const { dragIndex, dropIndex } = this.state;
    if (!accounts || accounts.length === 0) {
      return (
        <SettingsCard className="account-list empty">
          <div className="account-list-empty">{localized('No accounts yet.')}</div>
        </SettingsCard>
      );
    }
    return (
      <SettingsCard className="account-list">
        <div role="listbox" aria-label={localized('Accounts')} onDrop={this._onDrop}>
          {accounts.map((account, idx) => {
            const isSelected = selected && selected.id === account.id;
            return (
              <div
                key={account.id}
                role="option"
                aria-selected={isSelected}
                tabIndex={isSelected ? 0 : -1}
                draggable={accounts.length > 1}
                className={classnames('account-card', {
                  selected: isSelected,
                  dragging: dragIndex === idx,
                  'drop-before': dragIndex !== null && dropIndex === idx,
                  'drop-after':
                    dragIndex !== null && idx === accounts.length - 1 && dropIndex === idx + 1,
                })}
                onClick={() => this.props.onSelectAccount(account)}
                onKeyDown={(e) => this._onKeyDown(e, idx)}
                onDragStart={(e) => this._onDragStart(e, idx)}
                onDragOver={(e) => this._onDragOver(e, idx)}
                onDragEnd={this._onDragEnd}
              >
                <AccountAvatar account={account} />
                <div className="account-card-text">
                  <div className="account-card-name" dir="auto">
                    {account.label}
                  </div>
                  <div className="account-card-subtitle" dir="auto">
                    {account.emailAddress} · {account.displayProvider()}
                  </div>
                </div>
                <AccountStatus
                  account={account}
                  offline={offlineAccountIds.includes(account.id)}
                  syncing={syncingAccountIds.includes(account.id)}
                />
              </div>
            );
          })}
        </div>
      </SettingsCard>
    );
  }
}

export default PreferencesAccountList;
