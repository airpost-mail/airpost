import React from 'react';
import { shell } from 'electron';
import { localized } from '../../../intl/localize';
import * as Actions from '../../../data/actions';
import { SettingsRow, SettingsSwitch, StatusPill } from '../../../components/settings';
import { Button } from '../../../components/kit/button';
import {
  EngineUnitResult,
  EngineUnitStatus,
  GatewayPairing,
  GatewayStatus,
  McpHttpStatus,
  ServicesChannel,
} from '../../../platform/services';
import { invokeServices, onServiceChange, servicePill } from './services-client';

// Preferences > General > Services on this computer. Main runs and installs these
// (main/services/); the rows show their state and ask main for the actions.

const WEB_ACCESS_KEY = 'core.services.webAccess';

const isMac = process.platform === 'darwin';

interface EngineUnitState {
  status: EngineUnitStatus | null;
  busy: boolean;
  error: string | null;
}

/** The engine as a launchd agent or systemd user service (`tern engine install`). */
export class EngineUnitRow extends React.Component<Record<string, unknown>, EngineUnitState> {
  static displayName = 'EngineUnitRow';

  _mounted = false;
  state: EngineUnitState = { status: null, busy: false, error: null };

  async componentDidMount() {
    this._mounted = true;
    const status = await invokeServices<EngineUnitStatus>(ServicesChannel.EngineUnitStatus);
    if (this._mounted) this.setState({ status });
  }

  componentWillUnmount() {
    this._mounted = false;
  }

  _set = async (install: boolean) => {
    this.setState({ busy: true, error: null });
    const result = await invokeServices<EngineUnitResult>(ServicesChannel.EngineUnitSet, install);
    if (!this._mounted) return;
    this.setState({
      busy: false,
      status: result ? result.status : this.state.status,
      error: !result
        ? localized('Restart Tern to change this.')
        : result.ok
          ? null
          : localized('Couldn’t change it: %@', result.error || ''),
    });
  };

  _description() {
    const { status, error } = this.state;
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
    if (status?.installed) {
      if (status.joined) state = localized('It is running, and this window uses it.');
      else if (status.running) state = localized('It is running.');
      else state = localized('It starts when you quit Tern and when you log in.');
    }
    return (
      <>
        {[what, how, state].filter(Boolean).join(' ')}
        {error && <span className="command-line-error"> {error}</span>}
      </>
    );
  }

  _controls() {
    const { status, busy } = this.state;
    if (!status) return null;
    if (!status.available) return <StatusPill kind="off">{localized('Not included')}</StatusPill>;
    return (
      <Button variant="outline" disabled={busy} onClick={() => this._set(!status.installed)}>
        {status.installed ? localized('Uninstall') : localized('Install')}
      </Button>
    );
  }

  render() {
    const { status } = this.state;
    return (
      <SettingsRow
        label={localized('Keep Tern running in the background')}
        description={this._description()}
      >
        {status?.installed && (
          <StatusPill kind={status.running ? 'ok' : 'off'}>
            {status.running ? localized('Running') : localized('Installed')}
          </StatusPill>
        )}
        {this._controls()}
      </SettingsRow>
    );
  }
}

interface WebAccessState {
  status: GatewayStatus | null;
  enabled: boolean;
  pairing: GatewayPairing | null;
  pairingBusy: boolean;
}

/** `tern-gateway serve` with the web client, which main runs while web access is on. */
export class WebAccessRows extends React.Component<Record<string, unknown>, WebAccessState> {
  static displayName = 'WebAccessRows';

  _mounted = false;
  _unsubscribe: (() => void) | null = null;
  state: WebAccessState = {
    status: null,
    enabled: !!AppEnv.config.get(WEB_ACCESS_KEY),
    pairing: null,
    pairingBusy: false,
  };

  async componentDidMount() {
    this._mounted = true;
    this._unsubscribe = onServiceChange<GatewayStatus>('gateway', (status) =>
      this.setState({ status })
    );
    const status = await invokeServices<GatewayStatus>(ServicesChannel.GatewayStatus);
    if (this._mounted && status) this.setState({ status });
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._unsubscribe) this._unsubscribe();
  }

  _onToggle = (enabled: boolean) => {
    AppEnv.config.set(WEB_ACCESS_KEY, enabled);
    this.setState({ enabled, pairing: enabled ? this.state.pairing : null });
  };

  _onPair = async () => {
    this.setState({ pairingBusy: true });
    const pairing = await invokeServices<GatewayPairing>(ServicesChannel.GatewayPair);
    if (!this._mounted) return;
    this.setState({
      pairingBusy: false,
      pairing: pairing || { ok: false, error: localized('Restart Tern to pair a browser.') },
    });
  };

  _statusText(status: GatewayStatus | null) {
    if (!status) return null;
    if (!status.available) {
      return localized(
        'This copy of Tern doesn’t include the web client (tern-gateway and the web app); the official download does.'
      );
    }
    if (status.state === 'running') return localized('Open %@ in a browser.', status.url);
    if (status.state === 'error') {
      return localized('Couldn’t start: %@. Tern tries again shortly.', status.error);
    }
    if (status.state === 'starting') return localized('Starting…');
    return null;
  }

  _renderPairing() {
    const { pairing, pairingBusy } = this.state;
    let description: React.ReactNode = localized(
      'Each browser signs in once with a one-time code, valid for ten minutes.'
    );
    if (pairing?.ok) {
      description = (
        <>
          {localized('Code: %@.', pairing.code)}{' '}
          {pairing.url
            ? localized('Open the link in the browser to pair it.')
            : localized('Enter it on the gateway’s pairing page.')}
        </>
      );
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

  render() {
    const { status, enabled } = this.state;
    const label = localized('Web access from this computer');
    const available = !status || status.available;
    const pill = status && status.available && enabled ? servicePill(status) : null;
    const what = localized(
      'Use Tern in a browser on this computer. Tern runs its web server (tern-gateway) while it is open; other computers can’t reach it.'
    );
    const text = enabled || !available ? this._statusText(status) : null;
    return (
      <>
        <SettingsRow label={label} description={text ? `${what} ${text}` : what}>
          {pill && <StatusPill kind={pill.kind}>{pill.text}</StatusPill>}
          {available ? (
            <SettingsSwitch checked={enabled} label={label} onChange={this._onToggle} />
          ) : (
            <StatusPill kind="off">{localized('Not included')}</StatusPill>
          )}
        </SettingsRow>
        {enabled && status?.state === 'running' && this._renderPairing()}
      </>
    );
  }
}

interface McpHttpRowState {
  status: McpHttpStatus | null;
}

/** Preferences > MCP's HTTP server, as a status line; its controls stay in that tab. */
export class McpHttpRow extends React.Component<Record<string, unknown>, McpHttpRowState> {
  static displayName = 'McpHttpRow';

  _mounted = false;
  _unsubscribe: (() => void) | null = null;
  state: McpHttpRowState = { status: null };

  async componentDidMount() {
    this._mounted = true;
    this._unsubscribe = onServiceChange<McpHttpStatus>('mcp-http', (status) =>
      this.setState({ status })
    );
    const status = await invokeServices<McpHttpStatus>(ServicesChannel.McpHttpStatus);
    if (this._mounted && status) this.setState({ status });
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._unsubscribe) this._unsubscribe();
  }

  render() {
    const { status } = this.state;
    let text = localized('Off.');
    if (status?.enabled && status.state === 'running') {
      text = localized('Serving %@.', status.url);
    } else if (status?.enabled && status.state === 'error') {
      text = localized('Not running: %@', status.error);
    } else if (status?.enabled) {
      text = localized('On.');
    }
    const pill = status?.enabled ? servicePill(status) : null;
    return (
      <SettingsRow
        label={localized('MCP over HTTP')}
        description={
          <>
            {localized('For AI apps that connect to Tern’s MCP server (tern-mcp http).')} {text}{' '}
            <a onClick={() => Actions.switchPreferencesTab('Agents')}>
              {localized('Change in MCP')}
            </a>
          </>
        }
      >
        {pill && <StatusPill kind={pill.kind}>{pill.text}</StatusPill>}
      </SettingsRow>
    );
  }
}
