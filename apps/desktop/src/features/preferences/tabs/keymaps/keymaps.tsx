import React from 'react';
import path from 'path';
import fs from 'fs';

import { SettingsPage, SettingsSection, SettingsRow } from 'mailspring-component-kit';
import { localized } from 'mailspring-exports';

import displayedKeybindings from './displayed-keybindings';
import CommandItem from './command-item';
import { Disposable } from 'event-kit';

export default class PreferencesKeymaps extends React.Component<
  { config: any },
  { templates: string[]; bindings: { [command: string]: [] } }
> {
  static displayName = 'PreferencesKeymaps';

  _disposable?: Disposable;

  constructor(props) {
    super(props);
    this.state = {
      templates: [],
      bindings: this._getStateFromKeymaps(),
    };
    this._loadTemplates();
  }

  componentDidMount() {
    this._disposable = AppEnv.keymaps.onDidReloadKeymap(() => {
      this.setState({ bindings: this._getStateFromKeymaps() });
    });
  }

  componentWillUnmount() {
    this._disposable.dispose();
  }

  _getStateFromKeymaps() {
    const bindings: { [command: string]: [] } = {};
    for (const section of displayedKeybindings) {
      for (const [command] of section.items) {
        bindings[command] = AppEnv.keymaps.getBindingsForCommand(command) || [];
      }
    }
    return bindings;
  }

  _loadTemplates() {
    const templatesDir = path.join(AppEnv.getLoadSettings().resourcePath, 'keymaps', 'templates');
    fs.readdir(templatesDir, (err, files) => {
      if (!files || !(files instanceof Array)) return;
      let templates = files.filter((filename) => {
        return path.extname(filename) === '.json';
      });
      templates = templates.map((filename) => {
        return path.parse(filename).name;
      });
      this.setState({ templates: templates });
    });
  }

  _onShowUserKeymaps() {
    const keymapsFile = AppEnv.keymaps.getUserKeymapPath();
    if (!fs.existsSync(keymapsFile)) {
      fs.writeFileSync(keymapsFile, '{}');
    }
    require('@electron/remote').shell.showItemInFolder(keymapsFile);
  }

  _onDeleteUserKeymap() {
    const chosen = require('@electron/remote').dialog.showMessageBoxSync({
      type: 'info',
      message: localized('Restore the shortcut set?'),
      detail: localized('Every shortcut you changed goes back to what the set uses.'),
      buttons: [localized('Cancel'), localized('Restore')],
    });

    if (chosen === 1) {
      const keymapsFile = AppEnv.keymaps.getUserKeymapPath();
      fs.writeFileSync(keymapsFile, '{}');
    }
  }

  render() {
    const template = this.props.config.get('core.keymapTemplate');
    return (
      <SettingsPage
        className="preferences-shortcuts"
        title={localized('Shortcuts')}
        description={localized('Start from a set you know, then change any key.')}
      >
        <SettingsSection
          note={localized(
            'To change a shortcut, click it and press the new keys. Two keys pressed one after the other also work, like G then I.'
          )}
        >
          <SettingsRow
            label={localized('Shortcut set')}
            description={localized('Uses the keys of a mail app you already know.')}
          >
            <select
              className="settings-select"
              aria-label={localized('Shortcut set')}
              value={template}
              onChange={(event) => this.props.config.set('core.keymapTemplate', event.target.value)}
            >
              {this.state.templates.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </select>
          </SettingsRow>
          <SettingsRow
            label={localized('Your changes')}
            description={localized(
              'Shortcuts you change are saved in a file on top of the set. You can also edit that file yourself.'
            )}
          >
            <button className="btn" onClick={this._onShowUserKeymaps}>
              {localized('Edit in a file')}
            </button>
            <button className="btn" onClick={this._onDeleteUserKeymap}>
              {localized('Restore the set')}
            </button>
          </SettingsRow>
        </SettingsSection>

        {displayedKeybindings.map((section) => (
          <SettingsSection key={section.title} title={section.title}>
            {section.items.map(([command, label]) => (
              <CommandItem
                key={command}
                command={command}
                label={label}
                bindings={this.state.bindings[command]}
              />
            ))}
          </SettingsSection>
        ))}
      </SettingsPage>
    );
  }
}
