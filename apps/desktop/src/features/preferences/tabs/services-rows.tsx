import React from 'react';
import { shell } from 'electron';
import { localized } from '../../../intl/localize';
import { SettingsRow, SettingsSwitch, StatusPill } from '../../../components/settings';
import { Button } from '../../../components/kit/button';
import { ternBinary } from './plugins/tern-cli';
import { CommandLineRow } from './command-line-row';
import {
  GATEWAY_BIND,
  LocalServiceName,
  Pairing,
  Runner,
  STATUS_POLL_MS,
  ServiceStatuses,
  findService,
  pairBrowser,
  runServiceCommand,
  serviceStatuses,
  servicePill,
  webAccessMissing,
} from './local-services';

// Preferences > General > Services on this computer: the units `tern` installs next to the
// app (local-services.ts), and the `tern` command itself.

const isMac = process.platform === 'darwin';

interface Props {
  /** Runs tern; specs give a fake. */
  run?: Runner;
}

interface State {
  statuses: ServiceStatuses | null;
  busy: LocalServiceName | null;
  errors: { [name in LocalServiceName]?: string | null };
  pairing: Pairing | null;
  pairingBusy: boolean;
}

export class ServicesRows extends React.Component<Props, State> {
  static displayName = 'ServicesRows';

  _mounted = false;
  _poll: NodeJS.Timeout | null = null;
  state: State = { statuses: null, busy: null, errors: {}, pairing: null, pairingBusy: false };

  componentDidMount() {
    this._mounted = true;
    if (!ternBinary() && !this.props.run) return;
    this._refresh();
    this._poll = setInterval(this._refresh, STATUS_POLL_MS);
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._poll) clearInterval(this._poll);
  }

  _refresh = async () => {
    const statuses = await serviceStatuses(this.props.run);
    if (this._mounted) this.setState({ statuses });
  };

  _service(name: LocalServiceName) {
    return this.state.statuses ? findService(this.state.statuses, name) : null;
  }

  _run = async (name: LocalServiceName, action: 'install' | 'uninstall' | 'start') => {
    this.setState({ busy: name, errors: { ...this.state.errors, [name]: null } });
    const result = await runServiceCommand(name, action, { run: this.props.run });
    if (!this._mounted) return;
    if (name === 'gateway' && action === 'uninstall') this.setState({ pairing: null });
    this.setState({ busy: null, errors: { ...this.state.errors, [name]: result.error } });
    await this._refresh();
  };

  _onPair = async () => {
    this.setState({ pairingBusy: true });
    const pairing = await pairBrowser(this.props.run);
    if (this._mounted) this.setState({ pairingBusy: false, pairing });
  };

  _error(name: LocalServiceName) {
    const error = this.state.errors[name];
    return error ? (
      <span className="command-line-error"> {localized('Couldn’t change it: %@', error)}</span>
    ) : null;
  }

  /** The tern in this build can't list or run services. */
  _unavailable() {
    const { statuses } = this.state;
    if (!ternBinary() && !this.props.run)
      return localized('This copy of Tern doesn’t include tern.');
    if (statuses && !statuses.services) return statuses.error;
    return null;
  }

  _renderEngine() {
    const engine = this._service('engine');
    const unavailable = this._unavailable();
    const what = isMac
      ? localized(
          'Mail keeps syncing, scheduled sends go out and agents keep working after you quit Tern, from the time you log in. Closing the window or hiding the menu bar icon doesn’t need this.'
        )
      : localized(
          'Mail keeps syncing, scheduled sends go out and agents keep working after you quit Tern, from the time you log in, with no tray icon.'
        );
    const how = isMac
      ? localized('Tern installs its mail engine (tern-engine) as a launchd agent.')
      : localized('Tern installs its mail engine (tern-engine) as a systemd user service.');
    let state = '';
    if (engine?.installed) {
      state = engine.running
        ? localized('It is running.')
        : localized('It starts when you quit Tern and when you log in.');
    }
    return (
      <SettingsRow
        label={localized('Keep Tern running in the background')}
        description={
          <>
            {[what, how, unavailable || state].filter(Boolean).join(' ')}
            {this._error('engine')}
          </>
        }
      >
        {engine?.installed && (
          <StatusPill kind={engine.running ? 'ok' : 'off'}>
            {engine.running ? localized('Running') : localized('Installed')}
          </StatusPill>
        )}
        {this.state.statuses?.services && (
          <Button
            variant="outline"
            disabled={!!this.state.busy}
            onClick={() => this._run('engine', engine?.installed ? 'uninstall' : 'install')}
          >
            {engine?.installed ? localized('Uninstall') : localized('Install')}
          </Button>
        )}
      </SettingsRow>
    );
  }

  _renderPairing() {
    const { pairing, pairingBusy } = this.state;
    let description: React.ReactNode = localized(
      'Each browser signs in once with a one-time code, valid for ten minutes.'
    );
    if (pairing?.ok) {
      description = pairing.url
        ? localized('Code: %@. Open the link in the browser to pair it.', pairing.code)
        : localized('Code: %@. Enter it on the pairing page.', pairing.code);
    } else if (pairing) {
      description = (
        <span className="command-line-error">
          {localized('Couldn’t pair: %@', pairing.error || '')}
        </span>
      );
    }
    return (
      <SettingsRow label={localized('Pair a browser')} description={description}>
        {pairing?.ok && pairing.url && (
          <Button variant="outline" onClick={() => shell.openExternal(pairing.url as string)}>
            {localized('Open Link')}
          </Button>
        )}
        <Button variant="outline" disabled={pairingBusy} onClick={this._onPair}>
          {pairing?.ok ? localized('New Code') : localized('Pair a Browser')}
        </Button>
      </SettingsRow>
    );
  }

  _renderWebAccess() {
    const gateway = this._service('gateway');
    const missing = webAccessMissing();
    const unavailable = this._unavailable();
    const label = localized('Web access from this computer');
    const what = localized(
      'Use Tern in a browser on this computer. Tern installs its web server (tern-gateway) as a background service; other computers can’t reach it, and it needs Tern or the background engine running.'
    );
    let state: string | null = unavailable;
    if (missing.length) {
      state = localized(
        'This copy of Tern doesn’t include the web client; the official download does.'
      );
    } else if (gateway?.installed && gateway.running) {
      state = localized('Open %@ in a browser.', gateway.url || `http://${GATEWAY_BIND}`);
    } else if (gateway?.installed) {
      state = localized('It is installed but not running.');
    }
    const pill = gateway?.installed ? servicePill(gateway) : null;
    const canChange = !missing.length && !!this.state.statuses?.services;
    return (
      <>
        <SettingsRow
          label={label}
          description={
            <>
              {state ? `${what} ${state}` : what}
              {this._error('gateway')}
            </>
          }
        >
          {pill && <StatusPill kind={pill.kind}>{pill.text}</StatusPill>}
          {gateway?.installed && !gateway.running && (
            <Button
              variant="outline"
              disabled={!!this.state.busy}
              onClick={() => this._run('gateway', 'start')}
            >
              {localized('Start')}
            </Button>
          )}
          {missing.length ? (
            <StatusPill kind="off">{localized('Not included')}</StatusPill>
          ) : (
            <SettingsSwitch
              checked={!!gateway?.installed}
              disabled={!canChange || !!this.state.busy}
              label={label}
              onChange={(on) => this._run('gateway', on ? 'install' : 'uninstall')}
            />
          )}
        </SettingsRow>
        {gateway?.installed && gateway.running && this._renderPairing()}
      </>
    );
  }

  render() {
    return (
      <>
        {this._renderEngine()}
        {this._renderWebAccess()}
        <CommandLineRow />
      </>
    );
  }
}
