import path from 'path';
import React from 'react';
import { localized, AccountStore, CategoryStore, Account } from 'mailspring-exports';

import {
  ACCESS_LEVELS,
  AuditEntry,
  McpPolicy,
  TOKEN_FILE,
  defaultPolicy,
  isValidPort,
  migrateCoreMcpConfig,
  policyPath,
  readAuditLog,
  readEffectivePolicy,
  writePolicy,
} from './mcp-policy';
import {
  MCP_CLIENTS,
  McpClientId,
  TernMcpStatus,
  installClient,
  resultMessage,
  ternMcpBinary,
  ternMcpStatus,
  uninstallClient,
} from './tern-mcp';

// Preferences > Integrations > AI assistants: the policy tern-mcp reads (mcp.json), its
// status, client registration through the bundled tern-mcp, and its audit log.

const ACCESS_LEVEL_LABELS: { [level in (typeof ACCESS_LEVELS)[number]]: string } = {
  'read-only': 'Read only',
  'read-write': 'Read and change (drafts, archive, labels)',
  'read-write-send': 'Read, change and send',
};

interface Props {
  /** Edits these files instead of the clients' own configs; specs always give them. */
  configFiles?: { [client in McpClientId]?: string };
}

interface ClientResult {
  ok: boolean;
  message: string;
  /** The client has a `tern` entry tern-mcp didn't write; offer to replace it. */
  canForce: boolean;
}

interface State {
  policy: McpPolicy;
  policyError: string | null;
  policyErrorFile: string | null;
  portInput: string;
  status: TernMcpStatus | null;
  statusError: string | null;
  busyClient: McpClientId | null;
  results: { [client in McpClientId]?: ClientResult };
  expanded: { [accountId: string]: boolean };
  audit: AuditEntry[];
}

export function configDir() {
  return AppEnv.getConfigDirPath();
}

function formatTime(ts: number) {
  const diff = Math.floor((Date.now() - ts) / 1000);
  if (diff < 60) return localized('%@s ago', Math.max(diff, 0));
  if (diff < 3600) return localized('%@m ago', Math.floor(diff / 60));
  if (diff < 86400) return localized('%@h ago', Math.floor(diff / 3600));
  return new Date(ts).toLocaleDateString();
}

export class McpSection extends React.Component<Props, State> {
  static displayName = 'McpSection';

  _mounted = false;

  constructor(props: Props) {
    super(props);
    this.state = {
      policy: defaultPolicy(),
      policyError: null,
      policyErrorFile: null,
      portInput: '',
      status: null,
      statusError: null,
      busyClient: null,
      results: {},
      expanded: {},
      audit: [],
    };
  }

  componentDidMount() {
    this._mounted = true;
    try {
      migrateCoreMcpConfig(configDir());
    } catch (err) {
      console.warn(`[MCP] Could not move core.mcp to mcp.json: ${err.message}`);
    }
    this._reloadPolicy();
    this._reloadAudit();
    this._refreshStatus();
  }

  componentWillUnmount() {
    this._mounted = false;
  }

  _reloadPolicy() {
    const { policy, error, errorFile } = readEffectivePolicy(
      configDir(),
      AppEnv.config.get('core.mcp')
    );
    this.setState({
      policy,
      policyError: error,
      policyErrorFile: errorFile || null,
      portInput: String(policy.http.port),
    });
    return error ? null : policy;
  }

  _reloadAudit = () => {
    this.setState({ audit: readAuditLog(configDir()) });
  };

  _refreshStatus = async () => {
    if (!ternMcpBinary()) return;
    try {
      const status = await ternMcpStatus(configDir());
      if (this._mounted) this.setState({ status, statusError: null });
    } catch (err) {
      if (this._mounted) this.setState({ status: null, statusError: err.message });
    }
  };

  /** Applies `change` to the file as it is now, so edits made elsewhere while the pane is
   * open aren't reverted. */
  _save(change: (policy: McpPolicy) => McpPolicy) {
    if (this.state.policyError) return;
    const current = this._reloadPolicy();
    if (!current) return;
    try {
      writePolicy(configDir(), change(current));
    } catch (err) {
      this.setState({ policyError: err.message, policyErrorFile: policyPath(configDir()) });
      return;
    }
    this._reloadPolicy();
  }

  _onAccessLevel = (event: React.ChangeEvent<HTMLSelectElement>) => {
    const accessLevel = event.target.value;
    this._save((p) => ({ ...p, accessLevel }));
  };

  /** An empty map means every account, so the first change lists them all as enabled. */
  _accountsForChange(policy: McpPolicy) {
    const accounts = { ...policy.enabledAccounts };
    if (Object.keys(accounts).length === 0) {
      for (const a of AccountStore.accounts()) {
        accounts[a.id] = { enabled: true, excludedFolderIds: [] };
      }
    }
    return accounts;
  }

  _isAccountEnabled(accountId: string) {
    const { enabledAccounts } = this.state.policy;
    if (Object.keys(enabledAccounts).length === 0) return true;
    return !!enabledAccounts[accountId]?.enabled;
  }

  _onToggleAccount = (accountId: string) => {
    const enabled = !this._isAccountEnabled(accountId);
    this._save((p) => {
      const accounts = this._accountsForChange(p);
      accounts[accountId] = {
        excludedFolderIds: [],
        ...accounts[accountId],
        enabled,
      };
      return { ...p, enabledAccounts: accounts };
    });
  };

  _onToggleFolder = (accountId: string, folderId: string) => {
    this._save((p) => {
      const accounts = this._accountsForChange(p);
      const account = accounts[accountId] || { enabled: true, excludedFolderIds: [] };
      const excluded = account.excludedFolderIds.includes(folderId)
        ? account.excludedFolderIds.filter((f) => f !== folderId)
        : [...account.excludedFolderIds, folderId];
      accounts[accountId] = { ...account, excludedFolderIds: excluded };
      return { ...p, enabledAccounts: accounts };
    });
  };

  _onToggleHttp = () => {
    this._save((p) => ({ ...p, http: { ...p.http, enabled: !p.http.enabled } }));
  };

  _onCommitPort = () => {
    const port = Number(this.state.portInput);
    if (!isValidPort(port) || port === 0) {
      this.setState({ portInput: String(this.state.policy.http.port) });
      return;
    }
    this._save((p) => ({ ...p, http: { ...p.http, port } }));
  };

  _onRunClient = async (client: McpClientId, action: 'install' | 'uninstall', force = false) => {
    const configFile = this.props.configFiles?.[client];
    this.setState({ busyClient: client });
    let result: ClientResult;
    try {
      const run =
        action === 'install'
          ? await installClient(client, configDir(), { force, configFile })
          : await uninstallClient(client, configDir(), configFile);
      let message = resultMessage(run);
      if (run.ok && action === 'install' && message.startsWith('Added')) {
        message += ' ' + localized('Restart it and start a new chat to see Tern.');
      }
      result = {
        ok: run.ok,
        message,
        canForce: !run.ok && action === 'install' && message.includes('--force'),
      };
    } catch (err) {
      result = { ok: false, message: err.message, canForce: false };
    }
    if (!this._mounted) return;
    this.setState({ busyClient: null, results: { ...this.state.results, [client]: result } });
  };

  _renderStatus() {
    const { status, statusError } = this.state;
    if (!ternMcpBinary()) {
      return (
        <p className="mcp-status">
          {localized(
            'tern-mcp is not in this build of Tern. Build it with `cargo build --release -p tern-mcp` and set TERN_MCP_BIN.'
          )}
        </p>
      );
    }
    let text = localized('Checking…');
    if (statusError) text = statusError;
    if (status) {
      const accounts = status.engines.reduce((n, e) => n + e.accounts.length, 0);
      text =
        status.engines.length === 0
          ? localized('No engine running. Assistants can reach your mail while Tern is open.')
          : localized('Engines found: %1$@, with %2$@ accounts.', status.engines.length, accounts);
      if (status.policy?.error) text += ` ${status.policy.error}`;
    }
    return (
      <div className="item mcp-status-row">
        <span className="mcp-status">{text}</span>
        <button className="btn btn-small" onClick={this._refreshStatus}>
          {localized('Refresh')}
        </button>
      </div>
    );
  }

  _renderClients() {
    const { busyClient, results } = this.state;
    const available = !!ternMcpBinary();
    return MCP_CLIENTS.map(({ id, name }) => {
      const result = results[id];
      return (
        <div key={id} className="item mcp-client">
          <span className="mcp-client-name">{name}</span>
          <button
            className="btn btn-small"
            disabled={!available || !!busyClient}
            onClick={() => this._onRunClient(id, 'install')}
          >
            {localized('Add')}
          </button>
          <button
            className="btn btn-small"
            disabled={!available || !!busyClient}
            onClick={() => this._onRunClient(id, 'uninstall')}
          >
            {localized('Remove')}
          </button>
          {result?.canForce && (
            <button
              className="btn btn-small"
              disabled={!!busyClient}
              onClick={() => this._onRunClient(id, 'install', true)}
            >
              {localized('Replace It')}
            </button>
          )}
          {result && (
            <span className={`mcp-client-result ${result.ok ? 'ok' : 'failed'}`}>
              {result.message}
            </span>
          )}
        </div>
      );
    });
  }

  _renderAccount(account: Account) {
    const enabled = this._isAccountEnabled(account.id);
    const excluded = this.state.policy.enabledAccounts[account.id]?.excludedFolderIds || [];
    const expanded = !!this.state.expanded[account.id];
    const id = `mcp-account-${account.id}`;
    return (
      <div key={account.id} className="mcp-account">
        <div className="item">
          <input
            id={id}
            type="checkbox"
            checked={enabled}
            onChange={() => this._onToggleAccount(account.id)}
          />
          <label htmlFor={id}>{account.label || account.emailAddress}</label>
          {enabled && (
            <button
              className="btn btn-small"
              onClick={() =>
                this.setState({ expanded: { ...this.state.expanded, [account.id]: !expanded } })
              }
            >
              {excluded.length
                ? localized('Folders (%@ hidden)', excluded.length)
                : localized('Folders')}
            </button>
          )}
        </div>
        {enabled && expanded && (
          <div className="mcp-folders">
            {CategoryStore.categories(account.id).map((cat) => (
              <label key={cat.id} className="mcp-folder">
                <input
                  type="checkbox"
                  checked={!excluded.includes(cat.id)}
                  onChange={() => this._onToggleFolder(account.id, cat.id)}
                />
                {cat.displayName || cat.name}
              </label>
            ))}
          </div>
        )}
      </div>
    );
  }

  _renderAudit() {
    const { audit } = this.state;
    return (
      <div className="mcp-audit">
        <div className="item mcp-audit-header">
          <span>{localized('Recent activity')}</span>
          <button className="btn btn-small" onClick={this._reloadAudit}>
            {localized('Refresh')}
          </button>
        </div>
        {audit.length === 0 ? (
          <p className="mcp-audit-empty">{localized('No tool calls recorded yet.')}</p>
        ) : (
          <table className="mcp-audit-table">
            <tbody>
              {audit.map((entry, i) => (
                <tr key={i} className={`outcome-${entry.outcome || 'ok'}`}>
                  <td title={new Date(entry.timestamp).toLocaleString()}>
                    {formatTime(entry.timestamp)}
                  </td>
                  <td>{entry.toolName}</td>
                  <td>{entry.outcome}</td>
                  <td className="mcp-audit-params" title={entry.params}>
                    {entry.resultSummary || entry.params}
                  </td>
                  <td>{entry.client}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    );
  }

  render() {
    const { policy, policyError, policyErrorFile, portInput } = this.state;
    const dir = configDir();
    return (
      <section className="integrations-section integrations-mcp">
        <h6>{localized('AI assistants')}</h6>
        <p className="integrations-privacy">
          {localized(
            'Claude, Codex and other MCP clients can use your mail through tern-mcp, with the access you choose here. Each tool call is logged below.'
          )}
        </p>
        {this._renderStatus()}
        {this._renderClients()}
        {policyError ? (
          <p className="integrations-status">
            {localized(
              '%1$@ is not valid (%2$@). Fix or delete it.',
              policyErrorFile || policyPath(dir),
              policyError
            )}
          </p>
        ) : (
          <>
            <div className="item integrations-field">
              <label htmlFor="mcp-access-level">{localized('Access')}:</label>
              <select
                id="mcp-access-level"
                value={policy.accessLevel}
                onChange={this._onAccessLevel}
              >
                {!ACCESS_LEVELS.includes(policy.accessLevel as any) && (
                  <option value={policy.accessLevel}>{localized('Read only')}</option>
                )}
                {ACCESS_LEVELS.map((level) => (
                  <option key={level} value={level}>
                    {localized(ACCESS_LEVEL_LABELS[level])}
                  </option>
                ))}
              </select>
            </div>
            <div className="mcp-accounts">
              {AccountStore.accounts().map((a) => this._renderAccount(a))}
            </div>
            <div className="item">
              <input
                id="mcp-http-enabled"
                type="checkbox"
                checked={policy.http.enabled}
                onChange={this._onToggleHttp}
              />
              <label htmlFor="mcp-http-enabled">
                {localized('Allow HTTP clients on 127.0.0.1')}
              </label>
            </div>
            {policy.http.enabled && (
              <>
                <div className="item integrations-field">
                  <label htmlFor="mcp-http-port">{localized('Port')}:</label>
                  <input
                    id="mcp-http-port"
                    type="text"
                    inputMode="numeric"
                    value={portInput}
                    onChange={(e) => this.setState({ portInput: e.target.value })}
                    onBlur={this._onCommitPort}
                    onKeyDown={(e) => e.key === 'Enter' && this._onCommitPort()}
                  />
                </div>
                <p className="integrations-privacy">
                  {localized(
                    'Start the server with `tern-mcp http`. Clients send the bearer token in %@.',
                    path.join(dir, TOKEN_FILE)
                  )}
                </p>
              </>
            )}
          </>
        )}
        {this._renderAudit()}
      </section>
    );
  }
}
