import React from 'react';
import { Account } from '../../data/models/account';
import { localized } from '../../intl/localize';
import * as Actions from '../../data/actions';
import { ipcRenderer } from 'electron';
import { IconButton } from '../../components/kit/icon-button';
import * as AccountCommands from './account-commands';
import { ContextMenuItem, Menus } from '../../platform/app-api';

export default class AccountSwitcher extends React.Component<{
  accounts: Account[];
  sidebarAccountIds: string[];
}> {
  static displayName = 'AccountSwitcher';

  _makeMenuTemplate = () => {
    let template: ContextMenuItem[] = AccountCommands.menuTemplate(
      this.props.accounts,
      this.props.sidebarAccountIds,
      {
        clickHandlers: true,
      }
    ).map(({ label, type, checked, click, accelerator }) => ({
      label,
      type,
      checked,
      click,
      accelerator,
    }));
    template = template.concat([
      { type: 'separator' },
      { label: `${localized('Add Account')}...`, click: this._onAddAccount },
      { label: `${localized('Manage Accounts')}...`, click: this._onManageAccounts },
    ]);
    return template;
  };

  // Handlers

  _onAddAccount = () => {
    ipcRenderer.send('command', 'application:add-account');
  };

  _onManageAccounts = () => {
    Actions.switchPreferencesTab('Accounts');
    Actions.openPreferences();
  };

  _onShowMenu = () => {
    Menus.popup(this._makeMenuTemplate());
  };

  // The native menu opens on mouse down, like a menu bar item; Enter and Space open it from the
  // keyboard, and preventDefault keeps the button's own click from opening it twice.
  _onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      this._onShowMenu();
    }
  };

  render() {
    return (
      <IconButton
        className="account-switcher"
        size="sm"
        icon="account-switcher-dropdown.png"
        label={localized('Account switcher')}
        tooltip={false}
        aria-haspopup="menu"
        onMouseDown={this._onShowMenu}
        onKeyDown={this._onKeyDown}
      />
    );
  }
}
