/* eslint global-require: 0 */
import React, { Component } from 'react';
import classnames from 'classnames';
import { ipcRenderer } from 'electron';
import {
  SettingsSection,
  SettingsRow,
  SettingsCard,
  SettingsSwitch,
} from '../../../../components/settings';
import { Button } from '../../../../components/kit/button';
import { TextField } from '../../../../components/kit/text-field';
import { SettingsSelect } from '../../../../components/settings-select';
import { localized } from '../../../../intl/localize';
import RegExpUtils from '../../../../utils/regexp';
import { Account, AccountAutoaddress } from '../../../../data/models/account';
import { Dialogs, openStaticPage } from '../../../../platform/app-api';
import { writePrivateTempFile } from '../../../../utils/private-temp';
import { escapeHTML } from 'underscore.string';

/** One address the account sends from, shown as text until the user edits it. */
class AliasRow extends Component<
  { alias: string; onSave: (value: string) => void; onRemove: () => void },
  { editing: boolean; value: string }
> {
  state = { editing: false, value: this.props.alias };

  _save = () => {
    const value = this.state.value.trim();
    this.setState({ editing: false });
    if (value && value !== this.props.alias) this.props.onSave(value);
  };

  render() {
    const { alias, onRemove } = this.props;
    if (this.state.editing) {
      return (
        <div className="settings-row alias-input-row">
          <TextField
            autoFocus
            aria-label={localized('Address')}
            value={this.state.value}
            onChange={(e) => this.setState({ value: e.target.value })}
            onBlur={this._save}
            onKeyDown={(e) => {
              if (e.key === 'Enter') this._save();
              if (e.key === 'Escape') this.setState({ editing: false, value: alias });
            }}
          />
        </div>
      );
    }
    return (
      <SettingsRow label={<span dir="auto">{alias}</span>}>
        <Button
          variant="outline"
          onClick={() => this.setState({ editing: true, value: alias })}
          aria-label={localized('Edit %@', alias)}
        >
          {localized('Edit')}
        </Button>
        <Button variant="outline" onClick={onRemove} aria-label={localized('Remove %@', alias)}>
          {localized('Remove')}
        </Button>
      </SettingsRow>
    );
  }
}

class PreferencesAccountDetails extends Component<
  {
    account: Account;
    isOffline: boolean;
    onAccountUpdated: (account: Account, newAccount: Account) => void;
    onRemoveAccount?: (account: Account) => void;
  },
  {
    account: Account;
    newAlias: string;
  }
> {
  constructor(props: PreferencesAccountDetails['props']) {
    super(props);
    this.state = { account: props.account.clone(), newAlias: '' };
  }

  componentDidUpdate(prevProps: {
    account: Account;
    onAccountUpdated: (account: Account, newAccount: Account) => void;
  }) {
    if (prevProps.account !== this.props.account) {
      this.setState({ account: this.props.account.clone() });
    }
  }

  componentWillUnmount() {
    this._saveChanges();
  }

  // Helpers

  /**
   * @private Will transform any user input into alias format.
   * It will ignore any text after an email, if one is entered.
   * If no email is entered, it will use the account's email.
   * It will treat the text before the email as the name for the alias.
   * If no name is entered, it will use the account's name value.
   * @param {string} str - The string the user entered on the alias input
   * @param {object} [account=this.props.account] - The account object
   */
  _makeAlias(str: string, account = this.props.account) {
    const emailRegex = RegExpUtils.emailRegex();
    const match = emailRegex.exec(str);
    if (!match) {
      return `${str || account.name} <${account.emailAddress}>`;
    }
    const email = match[0];
    let name = str.slice(0, Math.max(0, match.index - 1));
    if (!name) {
      name = account.name || localized('No name provided');
    }
    name = name.trim();
    // TODO Sanitize the name string
    return `${name} <${email}>`;
  }

  _saveChanges = () => {
    this.props.onAccountUpdated(this.props.account, this.state.account);
  };

  _setState = (updates: Partial<Account>, callback = () => {}) => {
    const account = Object.assign(this.state.account.clone(), updates);
    this.setState({ account }, callback);
  };

  _setStateAndSave = (updates: Partial<Account>) => {
    this._setState(updates, () => {
      this._saveChanges();
    });
  };

  // Handlers
  _onAccountAutoaddressUpdated = (autoaddress: AccountAutoaddress) => {
    this._setState({ autoaddress });
  };

  _onAccountAliasCreated = (newAlias: string) => {
    const coercedAlias = this._makeAlias(newAlias);
    const aliases = this.state.account.aliases.concat([coercedAlias]);
    this._setStateAndSave({ aliases });
  };

  _onAccountAliasUpdated = (newAlias: string, alias: string, idx: number) => {
    const coercedAlias = this._makeAlias(newAlias);
    const aliases = this.state.account.aliases.slice();
    let defaultAlias = this.state.account.defaultAlias;
    if (defaultAlias === alias) {
      defaultAlias = coercedAlias;
    }
    aliases[idx] = coercedAlias;
    this._setStateAndSave({ aliases, defaultAlias });
  };

  _onAccountAliasRemoved = (alias: string, idx: number) => {
    const aliases = this.state.account.aliases.slice();
    let defaultAlias = this.state.account.defaultAlias;
    if (defaultAlias === alias) {
      defaultAlias = null;
    }
    aliases.splice(idx, 1);
    this._setStateAndSave({ aliases, defaultAlias });
  };

  _onAddAlias = () => {
    const value = this.state.newAlias.trim();
    if (!value) return;
    this.setState({ newAlias: '' });
    this._onAccountAliasCreated(value);
  };

  _onDefaultAliasSelected = (value: string) => {
    const defaultAlias = value === 'None' ? null : value;
    this._setStateAndSave({ defaultAlias });
  };

  // The engine keeps the secrets: onboarding asks for new ones (account.setSecrets).
  _onReconnect = () => {
    ipcRenderer.send('command', 'application:add-account', {
      reconnectAccountId: this.state.account.id,
    });
  };

  _onResetCache = () => {
    AppEnv.engineBridge.resetCacheForAccount(this.state.account);
  };

  _onAddSharedMailbox = () => {
    // Intentionally omits account secrets — the shared mailbox flow runs a fresh
    // OAuth sign-in and only needs the provider to route to the right settings page.
    ipcRenderer.send('command', 'application:add-account', {
      existingAccountJSON: this.state.account.toJSON(),
      o365SharedMailbox: true,
    });
  };

  // The engine restarts the account when its settings change, so this applies right away.
  _onSaveDraftsToServerChanged = (saveDraftsToServer: boolean) => {
    this._setStateAndSave({
      settings: { ...this.state.account.settings, save_drafts_to_server: saveDraftsToServer },
    });
  };

  _onManageContacts = () => {
    ipcRenderer.send('command', 'application:show-contacts', {});
  };

  _onSetColor = (colorChanged: Partial<Account>) => {
    // TODO: Ensure that the account color is updated in all places where it is displayed:
    // - src/features/composer/account-contact-field.tsx
    // - src/features/contacts/list.tsx
    // - src/features/preferences/tabs/accounts/list.tsx
    // - src/features/thread-list/participants.tsx
    // - src/components/outline-view.tsx
    this._setState(colorChanged);
  };

  _onResetColor = () => {
    this.state.account.color = '';
    this._saveChanges();
  };

  /** The remove confirmation is open; the window takes input meanwhile, so a second click is ignored. */
  _confirmingRemove = false;

  _onRemoveAccount = async () => {
    if (this._confirmingRemove) return;
    const { account } = this.props;
    this._confirmingRemove = true;
    let chosen: number;
    try {
      chosen = await Dialogs.ask(
        {
          type: 'warning',
          message: localized('Remove %@?', account.emailAddress),
          detail: localized(
            "Its mail is deleted from this computer. Your mail stays on your provider's server."
          ),
          buttons: [localized('Cancel'), localized('Remove')],
          defaultId: 0,
          cancelId: 0,
        },
        'none'
      );
    } finally {
      this._confirmingRemove = false;
    }
    if (chosen === 1) this.props.onRemoveAccount?.(account);
  };

  _onShowErrorDetails = async () => {
    const { id, syncState, settings, provider } = this.props.account;
    let filepath: string;
    try {
      const logs = await AppEnv.engineBridge.tailClientLog(id);
      const result = [
        `Airpost Version: ${AppEnv.getVersion()}`,
        `Platform: ${process.platform}`,
        `Account State: ${syncState}`,
        `Account Provider: ${provider}`,
        `IMAP Server: ${settings.imap_host}`,
        `SMTP Server: ${settings.smtp_host}`,
        `--------------------------------------------`,
        logs,
      ].join('\n');

      filepath = writePrivateTempFile(
        `error-details-${id}.html`,
        `<meta charset="UTF-8"><div style="white-space: pre-wrap; font-family: monospace;">${escapeHTML(
          result
        )}</div>`
      );
    } catch (err) {
      AppEnv.showErrorDialog({ title: 'Error', message: `Could not retrieve sync logs. ${err}` });
      return;
    }
    openStaticPage({
      filePath: filepath,
      title: `Account ${id} - Recent Logs`,
      width: 800,
      height: 600,
    });
  };

  // Renderers

  _renderBanner(
    kind: 'bad' | 'warn',
    title: string,
    detail: string,
    primary: { text: string; action: () => void }
  ) {
    return (
      <SettingsCard className={`account-banner ${kind}`}>
        <div className="account-banner-icon" aria-hidden>
          !
        </div>
        <div className="account-banner-text" role="alert">
          <div className="account-banner-title">{title}</div>
          <div className="account-banner-detail">{detail}</div>
        </div>
        <div className="account-banner-actions">
          <Button variant="outline" onClick={this._onShowErrorDetails}>
            {localized('Error details')}
          </Button>
          <Button variant="default" onClick={primary.action}>
            {primary.text}
          </Button>
        </div>
      </SettingsCard>
    );
  }

  _renderSyncErrorDetails() {
    const { account } = this.state;

    switch (account.syncState) {
      case Account.SYNC_STATE_AUTH_FAILED:
        return this._renderBanner(
          'bad',
          localized("Airpost can't sign in to %@", account.emailAddress),
          localized(
            'The password or sign-in may have changed. Mail for this account has stopped syncing until you reconnect.'
          ),
          { text: localized('Reconnect'), action: this._onReconnect }
        );
      case Account.SYNC_STATE_ERROR:
        return this._renderBanner(
          'bad',
          localized('Mail for %@ stopped syncing', account.emailAddress),
          localized(
            'Airpost ran into an error it could not get past. Reconnecting often fixes it; if it keeps happening, rebuild the account below.'
          ),
          { text: localized('Reconnect'), action: this._onReconnect }
        );
      default:
        if (!this.props.isOffline) {
          return null;
        }
        return this._renderBanner(
          'warn',
          localized("Airpost can't reach %@", account.settings.imap_host || account.emailAddress),
          localized(
            'It will keep trying. Your computer may be offline, or your mail provider may be having trouble.'
          ),
          { text: localized('Try now'), action: () => AppEnv.engineBridge.sendSyncMailNow() }
        );
    }
  }

  _renderConnectionRow() {
    const { account } = this.state;
    if (account.usesOAuth()) {
      return (
        <SettingsRow
          label={localized('Signed in with %@', account.displayProvider())}
          description={localized('Sign in again if Airpost stops getting your mail.')}
        >
          <Button variant="outline" onClick={this._onReconnect}>
            {localized('Sign in again…')}
          </Button>
        </SettingsRow>
      );
    }
    const host = account.settings.imap_host;
    return (
      <SettingsRow
        label={
          host
            ? localized('Connected to %@', host)
            : localized('Connected with %@', account.displayProvider())
        }
        description={localized('Change the password, servers or ports Airpost uses.')}
      >
        <Button variant="outline" onClick={this._onReconnect}>
          {localized('Update connection settings…')}
        </Button>
      </SettingsRow>
    );
  }

  render() {
    const { account, newAlias } = this.state;
    const aliasPlaceholder = this._makeAlias(`alias@${account.emailAddress.split('@')[1]}`);
    const aliases = account.aliases;
    const defaultAlias = account.defaultAlias || 'None';
    const { autoaddress } = account;

    return (
      <div className="account-details">
        {this._renderSyncErrorDetails()}

        <SettingsSection title={<span dir="auto">{account.label}</span>}>
          <SettingsRow label={localized('Name in the sidebar')}>
            <TextField
              id="account-label"
              aria-label={localized('Name in the sidebar')}
              value={account.label}
              onBlur={this._saveChanges}
              onChange={(e) => this._setState({ label: e.target.value })}
            />
          </SettingsRow>
          <SettingsRow label={localized('Your name on sent mail')}>
            <TextField
              id="account-sender-name"
              aria-label={localized('Your name on sent mail')}
              value={account.name}
              onBlur={this._saveChanges}
              onChange={(e) => this._setState({ name: e.target.value })}
            />
          </SettingsRow>
          <SettingsRow
            label={localized('Colour')}
            description={localized('Marks this account in the sidebar and message list.')}
          >
            <input
              id="account-color"
              type="color"
              aria-label={localized('Colour')}
              className={classnames({ unset: !account.color })}
              value={account.color || '#000000'}
              onBlur={this._saveChanges}
              onChange={(e) => this._onSetColor({ color: e.target.value })}
            />
            {account.color ? (
              <Button variant="outline" onClick={this._onResetColor}>
                {localized('No colour')}
              </Button>
            ) : (
              <span className="account-color-none">{localized('None')}</span>
            )}
          </SettingsRow>
          <SettingsRow
            label={localized('Always copy')}
            description={localized('Adds these addresses to every message you write.')}
            className="autoaddress-row"
          >
            <SettingsSelect
              aria-label={localized('Cc or Bcc')}
              value={autoaddress.type}
              onChange={(value) =>
                this._setStateAndSave({
                  autoaddress: { ...autoaddress, type: value === 'bcc' ? 'bcc' : 'cc' },
                })
              }
              options={[
                ['cc', localized('Cc')],
                ['bcc', localized('Bcc')],
              ]}
            />
            <TextField
              aria-label={localized('Addresses to copy, separated by commas')}
              value={autoaddress.value}
              onChange={(e) =>
                this._onAccountAutoaddressUpdated({ ...autoaddress, value: e.target.value })
              }
              onBlur={this._saveChanges}
              placeholder={localized('Addresses, comma-separated')}
            />
          </SettingsRow>
        </SettingsSection>

        <SettingsSection
          title={localized('Other addresses you send from')}
          note={localized(
            'Most providers only send from an address you have set up with them first.'
          )}
        >
          {aliases.map((alias, idx) => (
            <AliasRow
              key={`${idx}-${alias}`}
              alias={alias}
              onSave={(value) => this._onAccountAliasUpdated(value, alias, idx)}
              onRemove={() => this._onAccountAliasRemoved(alias, idx)}
            />
          ))}
          <div className="settings-row alias-input-row">
            <TextField
              aria-label={localized('New address')}
              placeholder={aliasPlaceholder}
              value={newAlias}
              onChange={(e) => this.setState({ newAlias: e.target.value })}
              onKeyDown={(e) => e.key === 'Enter' && this._onAddAlias()}
            />
            <Button variant="outline" disabled={!newAlias.trim()} onClick={this._onAddAlias}>
              {localized('Add')}
            </Button>
          </div>
          {aliases.length > 0 && (
            <SettingsRow label={localized('New messages from')}>
              <SettingsSelect
                aria-label={localized('New messages from')}
                value={defaultAlias}
                onChange={this._onDefaultAliasSelected}
                options={[
                  ['None', `${account.name} <${account.emailAddress}>`],
                  ...aliases.map((alias): [string, string] => [alias, alias]),
                ]}
              />
            </SettingsRow>
          )}
        </SettingsSection>

        <SettingsSection title={localized('Connection')}>
          {this._renderConnectionRow()}
          {account.provider === 'office365' && (
            <SettingsRow
              label={localized('Shared mailbox')}
              description={localized('Add a mailbox other people in your organisation share.')}
            >
              <Button variant="outline" onClick={this._onAddSharedMailbox}>
                {localized('Add shared mailbox…')}
              </Button>
            </SettingsRow>
          )}
          <SettingsRow
            label={localized('Save drafts to server')}
            description={localized(
              "Drafts are saved to the account's Drafts folder, so they show up in webmail and on your other devices."
            )}
          >
            <SettingsSwitch
              label={localized('Save drafts to server')}
              checked={account.settings.save_drafts_to_server !== false}
              onChange={this._onSaveDraftsToServerChanged}
            />
          </SettingsRow>
          <SettingsRow label={localized('Contacts')}>
            <Button variant="outline" onClick={this._onManageContacts}>
              {localized('Manage contacts')}
            </Button>
          </SettingsRow>
        </SettingsSection>

        <SettingsSection title={localized('On this computer')}>
          <SettingsRow
            label={localized('Rebuild this account')}
            description={localized(
              'Downloads its mail again. Use it when mail is missing or looks wrong. It can take a while for a large mailbox.'
            )}
          >
            <Button variant="outline" onClick={this._onResetCache}>
              {localized('Rebuild…')}
            </Button>
          </SettingsRow>
          <SettingsRow
            label={localized('Remove this account')}
            description={localized(
              "Deletes its mail from this computer. Your mail stays on your provider's server."
            )}
          >
            <Button variant="destructive" onClick={this._onRemoveAccount}>
              {localized('Remove…')}
            </Button>
          </SettingsRow>
        </SettingsSection>
      </div>
    );
  }
}

export default PreferencesAccountDetails;
