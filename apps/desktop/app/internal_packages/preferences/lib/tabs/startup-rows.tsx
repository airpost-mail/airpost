import React from 'react';
import { localized, DefaultClientHelper, SystemStartService } from 'mailspring-exports';
import { shell } from 'electron';

import { SettingsRow, SettingsSwitch } from 'mailspring-component-kit';

interface DefaultMailClientItemState {
  defaultClient: boolean | 'unknown';
}

const DELAY_FOR_SHEET_ANIMATION = 25;

const helper = new DefaultClientHelper();
const service = new SystemStartService();

export class DefaultMailClientRow extends React.Component<
  Record<string, unknown>,
  DefaultMailClientItemState
> {
  _mounted = false;
  state: DefaultMailClientItemState = { defaultClient: helper.available() ? false : 'unknown' };

  async componentDidMount() {
    this._mounted = true;
    if (helper.available()) {
      await Promise.delay(DELAY_FOR_SHEET_ANIMATION);
      if (!this._mounted) return;
      helper.isRegisteredForURLScheme('mailto', (registered) => {
        if (!this._mounted) return;
        this.setState({ defaultClient: registered instanceof Error ? 'unknown' : registered });
      });
    }
  }

  componentWillUnmount() {
    this._mounted = false;
  }

  _onMakeDefault = () => {
    if (this.state.defaultClient === 'unknown') {
      if (process.platform === 'win32') {
        helper.registerForURLScheme('mailto');
      } else {
        shell.openExternal('https://github.com/ternmail/ternmail#readme');
      }
      return;
    }
    this.setState({ defaultClient: true });
    helper.registerForURLScheme('mailto');
  };

  _onReset = () => {
    this.setState({ defaultClient: false });
    helper.resetURLScheme('mailto');
  };

  render() {
    if (this.state.defaultClient === true) {
      return (
        <SettingsRow
          label={localized('Tern is your default mail app')}
          description={localized('Mail links in other apps open in Tern.')}
        >
          <button className="btn" onClick={this._onReset}>
            {localized('Stop using Tern')}
          </button>
        </SettingsRow>
      );
    }
    return (
      <SettingsRow
        label={localized("Tern isn't your default mail app")}
        description={localized('Mail links in other apps open in another mail app.')}
      >
        <button className="btn" onClick={this._onMakeDefault}>
          {localized('Make Tern the default')}
        </button>
      </SettingsRow>
    );
  }
}

interface LaunchSystemStartItemState {
  launchOnStart: boolean | 'unavailable';
}

export class LaunchOnStartRow extends React.Component<
  Record<string, unknown>,
  LaunchSystemStartItemState
> {
  _mounted = false;
  state: LaunchSystemStartItemState = { launchOnStart: 'unavailable' };

  async componentDidMount() {
    this._mounted = true;
    const available = await service.checkAvailability();
    if (available && this._mounted) {
      const launchOnStart = await service.doesLaunchOnSystemStart();
      if (!this._mounted) return;
      this.setState({ launchOnStart: launchOnStart });
    } else if (this._mounted) {
      this.setState({ launchOnStart: 'unavailable' });
    }
  }

  componentWillUnmount() {
    this._mounted = false;
  }

  _toggle = (next: boolean) => {
    this.setState({ launchOnStart: next });
    if (next) {
      service.configureToLaunchOnSystemStart();
    } else {
      service.dontLaunchOnSystemStart();
    }
  };

  render() {
    if (this.state.launchOnStart === 'unavailable') return null;
    return (
      <SettingsRow
        label={localized('Open Tern when you log in')}
        description={
          process.platform === 'linux'
            ? localized('Works in XDG-compliant desktop environments.')
            : undefined
        }
      >
        <SettingsSwitch
          checked={this.state.launchOnStart}
          label={localized('Open Tern when you log in')}
          onChange={this._toggle}
        />
      </SettingsRow>
    );
  }
}
