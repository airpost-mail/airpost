import os from 'os';
import path from 'path';
import React from 'react';
import { ipcRenderer } from 'electron';
import { localized } from '../../../intl/localize';
import { SettingsRow, StatusPill } from '../../../components/settings';
import { Button } from '../../../components/kit/button';
import { CliLinkChannel, CliLinkResult, CliLinkStatus } from '../../../platform/cli-link';
import { ternBinary } from './plugins/tern-cli';

interface State {
  status: CliLinkStatus | null;
  busy: boolean;
  error: string | null;
}

/** The `tern` command: installs it on the PATH as a link to this copy's binary (main/cli-link.ts). */
export class CommandLineRow extends React.Component<Record<string, unknown>, State> {
  static displayName = 'CommandLineRow';

  _mounted = false;
  state: State = { status: null, busy: false, error: null };

  async componentDidMount() {
    this._mounted = true;
    if (!ternBinary()) return;
    try {
      const status: CliLinkStatus | null = await ipcRenderer.invoke(CliLinkChannel.Status);
      if (this._mounted) this.setState({ status });
    } catch (err) {
      // No handler: main is older than this window, as after a reload without a restart.
      if (this._mounted) this.setState({ error: localized('Restart Tern to install it.') });
    }
  }

  componentWillUnmount() {
    this._mounted = false;
  }

  _run = async (channel: string, ...args: unknown[]) => {
    this.setState({ busy: true, error: null });
    let result: CliLinkResult | null;
    try {
      result = await ipcRenderer.invoke(channel, ...args);
    } catch (err) {
      if (this._mounted) this.setState({ busy: false, error: (err as Error).message });
      return;
    }
    if (!this._mounted) return;
    this.setState({
      busy: false,
      status: result ? result.status : this.state.status,
      error: result && !result.ok && !result.cancelled ? this._errorText(result) : null,
    });
  };

  _errorText(result: CliLinkResult) {
    if (result.error === 'needs-move') return localized('Move Tern to Applications first.');
    if (result.error === 'taken') return null;
    return localized('Couldn’t install it: %@', result.error || '');
  }

  _onInstall = () => this._run(CliLinkChannel.Install, false);
  _onReplace = () => this._run(CliLinkChannel.Install, true);
  _onUninstall = () => this._run(CliLinkChannel.Uninstall);

  _description() {
    const { status, error } = this.state;
    const what = localized('Search, read and send mail from the terminal and your own scripts.');
    const tern = ternBinary();
    if (!tern) {
      return `${what} ${localized('This copy of Tern doesn’t include it; the official download does.')}`;
    }
    let text = `${what} ${localized('It comes with Tern, at %@.', tern)}`;
    if (status?.state === 'installed') {
      text = `${what} ${localized('Installed at %@.', status.path)}`;
      if (status.profile) text += ` ${localized('It uses this profile, %@.', status.profile)}`;
      // ~/.local/bin isn't on every shell's PATH; /usr/local/bin is on macOS's default one.
      if (status.path?.startsWith(os.homedir())) {
        text += ` ${localized('Make sure %@ is on your PATH.', path.dirname(status.path))}`;
      }
    } else if (status?.state === 'missing') {
      text = status.needsMove
        ? `${what} ${localized('Move Tern to Applications to install it.')}`
        : `${what} ${localized('Install puts it at %@.', status.path)}`;
    } else if (status?.state === 'other') {
      if (!status.replaceable) {
        text = `${what} ${localized('Another program is already at %@, so Tern leaves it alone.', status.path)}`;
      } else if (status.sameBinary) {
        text = `${what} ${localized('%@ runs this copy of Tern with another profile.', status.path)}`;
      } else {
        text = `${what} ${localized('%1$@ points to another copy, at %2$@.', status.path, status.otherTarget)}`;
      }
    }
    return (
      <>
        {text}
        {error && <span className="command-line-error"> {error}</span>}
      </>
    );
  }

  _controls() {
    const { status, busy } = this.state;
    if (!ternBinary()) return <StatusPill kind="off">{localized('Not included')}</StatusPill>;
    switch (status?.state) {
      case 'installed':
        return (
          <Button variant="outline" disabled={busy} onClick={this._onUninstall}>
            {localized('Uninstall')}
          </Button>
        );
      case 'missing':
        return (
          <Button variant="outline" disabled={busy || !!status.needsMove} onClick={this._onInstall}>
            {localized('Install')}
          </Button>
        );
      case 'other':
        return status.replaceable ? (
          <Button variant="outline" disabled={busy} onClick={this._onReplace}>
            {localized('Replace It')}
          </Button>
        ) : (
          <StatusPill kind="warn">{localized('Not installed')}</StatusPill>
        );
      default:
        return <StatusPill kind="ok">{localized('Included')}</StatusPill>;
    }
  }

  render() {
    return (
      <SettingsRow label={localized('tern command')} description={this._description()}>
        {this._controls()}
      </SettingsRow>
    );
  }
}
