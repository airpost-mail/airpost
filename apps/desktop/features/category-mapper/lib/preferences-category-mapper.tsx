import React from 'react';
import {
  localized,
  Account,
  AccountStore,
  CategoryStore,
  Category,
  Actions,
  ChangeRoleMappingTask,
  Folder,
  imapUtf7,
} from 'mailspring-exports';
import { SettingsPage, SettingsSection, SettingsRow } from 'mailspring-component-kit';

const SELECTABLE_ROLES = ['inbox', 'sent', 'drafts', 'spam', 'archive', 'trash'];

interface State {
  assignments: {
    [accountId: string]: {
      [role: string]: Category;
    };
  };
  all: {
    [accountId: string]: Category[];
  };
  containerFolderDefault: string;
}

export default class PreferencesCategoryMapper extends React.Component<
  Record<string, unknown>,
  State
> {
  _unlisten?: () => void;

  constructor(props) {
    super(props);
    this.state = this._getStateFromStores();
  }

  componentDidMount() {
    this._unlisten = CategoryStore.listen(() => {
      this.setState(this._getStateFromStores());
    });
  }

  componentWillUnmount() {
    if (this._unlisten) {
      this._unlisten();
    }
  }

  _getStateFromStores() {
    const assignments = {};
    const all = {};

    for (const cat of CategoryStore.categories()) {
      all[cat.accountId] = all[cat.accountId] || [];
      all[cat.accountId].push(cat);
      if (SELECTABLE_ROLES.includes(cat.role)) {
        assignments[cat.accountId] = assignments[cat.accountId] || {};
        assignments[cat.accountId][cat.role] = cat;
      }
    }

    const containerFolderDefault = AccountStore.containerFolderDefaultGetter();
    return { assignments, all, containerFolderDefault };
  }

  _onCategorySelection = async (account, role, category) => {
    // our state will be updated as soon as the sync worker commits the change
    Actions.queueTask(
      new ChangeRoleMappingTask({
        role: role,
        path: category.path,
        accountId: account.id,
      })
    );
  };

  _updateContainerFolderDefault = () => {
    Actions.updateContainerFolderDefault(this.state.containerFolderDefault);
  };

  _renderRoleRow = (account: Account, role: string) => {
    if (account.provider === 'gmail' && role === 'archive') {
      return false;
    }

    const allowLabels = account.usesLabels() && role !== 'trash' && role !== 'spam';
    const choices = (this.state.all[account.id] || [])
      .filter((c) => allowLabels || c instanceof Folder)
      .map((c) => ({ category: c, path: imapUtf7.decode(c.path) }))
      .sort((a, b) => a.path.toUpperCase().localeCompare(b.path.toUpperCase()));
    const current = this.state.assignments[account.id][role];
    const label = Category.LocalizedStringForRole[role];

    return (
      <SettingsRow key={role} label={label}>
        <select
          className="settings-select"
          aria-label={label}
          value={current ? current.id : ''}
          onChange={(e) => {
            const picked = choices.find((c) => c.category.id === e.target.value);
            if (picked) this._onCategorySelection(account, role, picked.category);
          }}
        >
          {!current && (
            <option value="" disabled>
              {localized('None')}
            </option>
          )}
          {choices.map(({ category, path }) => (
            <option key={category.id} value={category.id}>
              {path}
            </option>
          ))}
        </select>
      </SettingsRow>
    );
  };

  _renderAccount = (account: Account) => {
    // An account that hasn't finished its first sync has no folders with roles yet.
    const synced = !!this.state.assignments[account.id];
    return (
      <SettingsSection key={account.id} title={account.label}>
        {synced ? (
          SELECTABLE_ROLES.map((role) => this._renderRoleRow(account, role))
        ) : (
          <SettingsRow
            label={localized('Not synced yet')}
            description={localized('The folders show up here once Tern has synced this account.')}
          />
        )}
      </SettingsSection>
    );
  };

  render() {
    return (
      <SettingsPage
        title={localized('Folders')}
        description={localized(
          'Which folder each account uses for your inbox, sent mail, drafts, archive, spam and trash.'
        )}
      >
        {AccountStore.accounts().map(this._renderAccount)}
        <SettingsSection title={localized('New accounts')}>
          <SettingsRow
            label={localized("Folder for Tern's folders")}
            description={localized(
              'Tern keeps the folders it makes, like Snoozed, inside this folder in accounts you add from now on. Use folder/subfolder for a subfolder.'
            )}
          >
            <input
              type="text"
              className="category-mapper-container-folder"
              aria-label={localized("Folder for Tern's folders")}
              value={this.state.containerFolderDefault}
              onBlur={this._updateContainerFolderDefault}
              onChange={(e) => this.setState({ containerFolderDefault: e.target.value })}
            />
          </SettingsRow>
        </SettingsSection>
      </SettingsPage>
    );
  }
}
