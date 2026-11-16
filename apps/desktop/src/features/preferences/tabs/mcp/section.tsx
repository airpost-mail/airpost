import fs from 'fs';
import path from 'path';
import React from 'react';
import classNames from 'classnames';
import { localized } from '../../../../intl/localize';
import { AccountStore } from '../../../../data/stores/account';
import CategoryStore from '../../../../data/stores/category';
import { Account } from '../../../../data/models/account';
import * as Actions from '../../../../data/actions';
import {
  SettingsSection,
  SettingsRow,
  SettingsSwitch,
  SettingsNote,
  StatusPill,
  ProviderCard,
  ProviderLogo,
} from '../../../../components/settings';
import { Button } from '../../../../components/kit/button';
import { Checkbox } from '../../../../components/kit/checkbox';
import { TextField } from '../../../../components/kit/text-field';
import { SettingsSelect } from '../../../../components/settings-select';
import { Clipboard } from '../../../../platform/app-api';
import {
  LocalService,
  Runner,
  STATUS_POLL_MS,
  findService,
  runServiceCommand,
  serviceStatuses,
  servicePill,
} from '../local-services';

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
} from './policy';
import {
  MCP_CLIENTS,
  McpClientId,
  AirpostMcpStatus,
  installClient,
  resultMessage,
  airpostMcpBinary,
  airpostMcpStatus,
  uninstallClient,
} from './airpost-mcp';

// Preferences > MCP: the policy airpost-mcp reads (mcp.json), client registration through the
// bundled airpost-mcp, the `airpost-mcp http` server (a user unit `airpost mcp-http install` puts in
// place, local-services.ts), and the audit log and status. The `airpost` command is in General.

type AccessLevel = (typeof ACCESS_LEVELS)[number];

const ACCESS_LEVEL_LABELS: { [level in AccessLevel]: string } = {
  'read-only': 'Read mail',
  'read-write': 'Read and draft',
  'read-write-send': 'Read, draft and send',
};

const ACCESS_LEVEL_DESCRIPTIONS: { [level in AccessLevel]: string } = {
  'read-only': "Apps can search and read your mail. They can't change anything.",
  'read-write':
    "Apps can also write drafts, archive, move, label and snooze mail, and edit events and contacts. They can't send.",
  'read-write-send': 'Apps can also send mail, unsubscribe and answer invitations.',
};

const CLIENT_LOGOS: { [client in McpClientId]: { text: string; color: string } } = {
  'claude-desktop': { text: 'C', color: '#c96442' },
  'claude-code': { text: 'CC', color: '#d97757' },
  codex: { text: 'Cx', color: '#10a37f' },
};

// The MCP `clientInfo.name` each app sends, which airpost-mcp writes to the audit log.
const CLIENT_INFO_NAMES: { [client in McpClientId]: RegExp } = {
  'claude-desktop': /^claude-ai\b/,
  'claude-code': /^claude-code\b/,
  codex: /^codex/,
};

const TOOL_PHRASES: { [tool: string]: string } = {
  list_accounts: 'listed your accounts',
  list_folders: 'listed your folders',
  search_mail: 'searched your mail',
  list_threads: 'listed conversations',
  get_thread: 'read a conversation',
  get_message: 'read a message',
  get_message_tracking: 'checked if a message was opened',
  list_tracked_messages: 'listed tracked messages',
  get_top_links: 'listed the most clicked links',
  list_scheduled_sends: 'listed scheduled messages',
  get_attachment: 'opened an attachment',
  'resources/read': 'opened an attachment',
  create_draft: 'wrote a draft',
  reply_to_thread: 'drafted a reply',
  update_draft: 'changed a draft',
  delete_draft: 'deleted a draft',
  archive_threads: 'archived mail',
  trash_threads: 'moved mail to the trash',
  move_to_folder: 'moved mail to a folder',
  set_labels: 'changed labels',
  create_label: 'created a label',
  set_unread: 'marked mail read or unread',
  set_starred: 'starred or unstarred mail',
  undo: 'took back a change',
  set_reminder: 'set a reminder',
  send_draft: 'sent a message',
  schedule_send: 'scheduled a message',
  cancel_scheduled_send: 'cancelled a scheduled message',
  snooze_threads: 'snoozed mail',
  unsnooze_threads: 'unsnoozed mail',
  unsubscribe: 'unsubscribed from a list',
  list_events: 'listed calendar events',
  create_event: 'created an event',
  update_event: 'changed an event',
  respond_to_event: 'answered an invitation',
  get_contact: 'looked up a contact',
  create_contact: 'added a contact',
  update_contact: 'changed a contact',
};

const ACTIVITY_SHOWN = 10;

interface Props {
  /** Edits these files instead of the clients' own configs; specs always give them. */
  configFiles?: { [client in McpClientId]?: string };
  /** Runs airpost for the HTTP server; specs give a fake. */
  run?: Runner;
}

interface ClientResult {
  ok: boolean;
  message: string;
  /** Whether the app has Airpost after this action; null when the action failed. */
  connected: boolean | null;
  /** The client has a `airpost` entry airpost-mcp didn't write; offer to replace it. */
  canForce: boolean;
}

interface State {
  policy: McpPolicy;
  policyError: string | null;
  policyErrorFile: string | null;
  portInput: string;
  status: AirpostMcpStatus | null;
  statusError: string | null;
  busyClient: McpClientId | null;
  results: { [client in McpClientId]?: ClientResult };
  expanded: { [accountId: string]: boolean };
  audit: AuditEntry[];
  showAllActivity: boolean;
  trayTurnedOn: boolean;
  /** The `mcp-http` unit, as `airpost --json status` reports it. */
  http: LocalService | null;
  /** Why airpost couldn't report or change the server. */
  httpError: string | null;
  httpBusy: boolean;
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

function clientForEntry(entry: AuditEntry) {
  const name = entry.client || '';
  return MCP_CLIENTS.find((c) => CLIENT_INFO_NAMES[c.id].test(name));
}

function errorText(entry: AuditEntry) {
  return (entry.resultSummary || '').replace(/^error: /, '');
}

/** airpost-mcp's refusals (policy.rs `check_access`) in the pane's words. */
function blockedText(entry: AuditEntry) {
  const error = errorText(entry);
  if (error.includes('not allowed to use Airpost')) return localized('apps were turned off.');
  const needed = /require access level '([\w-]+)'/.exec(error)?.[1] as AccessLevel | undefined;
  if (needed && ACCESS_LEVEL_LABELS[needed]) {
    return localized('it needs “%@” access.', localized(ACCESS_LEVEL_LABELS[needed]));
  }
  return error;
}

/** The call's arguments as plain values, e.g. a search query; airpost-mcp has already
 * shortened bodies to their length. */
function paramsText(params: string | undefined) {
  if (!params) return null;
  try {
    const parsed = JSON.parse(params);
    if (!parsed || typeof parsed !== 'object') return params;
    const values = Object.values(parsed).filter(
      (v) => typeof v === 'string' || typeof v === 'number'
    );
    return values.length ? values.join(' · ') : null;
  } catch (err) {
    return params;
  }
}

/** Turns inertness on and off on the wrapped element: React 17 doesn't pass `inert`. */
function Dimmed(props: { off: boolean; className?: string; children: React.ReactNode }) {
  const ref = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    if (!ref.current) return;
    if (props.off) ref.current.setAttribute('inert', '');
    else ref.current.removeAttribute('inert');
  }, [props.off]);
  return (
    <div ref={ref} className={classNames(props.className || 'agents-body', { off: props.off })}>
      {props.children}
    </div>
  );
}

export class McpSection extends React.Component<Props, State> {
  static displayName = 'McpSection';

  _mounted = false;
  _poll: NodeJS.Timeout | null = null;

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
      showAllActivity: false,
      trayTurnedOn: false,
      http: null,
      httpError: null,
      httpBusy: false,
    };
  }

  componentDidMount() {
    this._mounted = true;
    try {
      migrateCoreMcpConfig(configDir());
    } catch (err) {
      console.warn(`[MCP] Could not move core.mcp to mcp.json: ${(err as Error).message}`);
    }
    this._reloadPolicy();
    this._reloadAudit();
    this._refreshStatus();
    this._refreshHttp();
    this._poll = setInterval(this._refreshHttp, STATUS_POLL_MS);
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._poll) clearInterval(this._poll);
  }

  _refreshHttp = async () => {
    const statuses = await serviceStatuses(this.props.run);
    if (!this._mounted) return;
    this.setState({
      http: findService(statuses, 'mcp-http'),
      httpError: statuses.services ? null : statuses.error,
    });
  };

  /** Installs the unit on `port`, or removes it. airpost-mcp http also refuses to serve while
   * http.enabled is off, so the switch writes mcp.json first. */
  async _applyHttp(on: boolean, port: number) {
    this.setState({ httpBusy: true, httpError: null });
    const result = await runServiceCommand('mcp-http', on ? 'install' : 'uninstall', {
      port,
      run: this.props.run,
    });
    if (!this._mounted) return;
    this.setState({ httpBusy: false });
    await this._refreshHttp();
    if (this._mounted && !result.ok) this.setState({ httpError: result.error });
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
    if (!airpostMcpBinary()) return;
    try {
      const status = await airpostMcpStatus(configDir());
      if (this._mounted) this.setState({ status, statusError: null });
    } catch (err) {
      if (this._mounted) this.setState({ status: null, statusError: (err as Error).message });
    }
  };

  /** Applies `change` to the file as it is now, so edits made elsewhere while the pane is
   * open aren't reverted. */
  _save(change: (policy: McpPolicy) => McpPolicy) {
    if (this.state.policyError) return false;
    const current = this._reloadPolicy();
    if (!current) return false;
    try {
      writePolicy(configDir(), change(current));
    } catch (err) {
      this.setState({
        policyError: (err as Error).message,
        policyErrorFile: policyPath(configDir()),
      });
      return false;
    }
    this._reloadPolicy();
    return true;
  }

  _isOn() {
    return this.state.policy.enabled !== false;
  }

  /** Apps only reach mail while the engine runs, and off macOS the tray icon is what keeps
   * Airpost running once the window closes (product-model.md section 3). */
  _keepRunning(on: boolean) {
    if (on && !AppEnv.config.get('core.workspace.systemTray')) {
      AppEnv.config.set('core.workspace.systemTray', true);
      this.setState({ trayTurnedOn: true });
    } else if (!on && !this._isOn() && !this.state.policy.http.enabled) {
      this.setState({ trayTurnedOn: false });
    }
  }

  _onToggleAccess = (on: boolean) => {
    if (!this._save((p) => ({ ...p, enabled: on }))) return;
    this._keepRunning(on);
  };

  _onAccessLevel = (accessLevel: string) => {
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
      const existing = Object.hasOwn(accounts, accountId) ? accounts[accountId] : undefined;
      accounts[accountId] = {
        excludedFolderIds: [],
        ...existing,
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
    const on = !this.state.policy.http.enabled;
    if (!this._save((p) => ({ ...p, http: { ...p.http, enabled: on } }))) return;
    this._keepRunning(on);
    this._applyHttp(on, this.state.policy.http.port);
  };

  _onCopyKey = () => {
    try {
      Clipboard.writeText(fs.readFileSync(path.join(configDir(), TOKEN_FILE), 'utf8').trim());
    } catch (err) {
      // airpost-mcp writes the key the first time the server starts.
    }
  };

  _onCommitPort = () => {
    const port = Number(this.state.portInput);
    if (!isValidPort(port) || port === 0) {
      this.setState({ portInput: String(this.state.policy.http.port) });
      return;
    }
    if (port === this.state.policy.http.port) return;
    if (this._save((p) => ({ ...p, http: { ...p.http, port } }))) this._applyHttp(true, port);
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
        message += ' ' + localized('Restart it and start a new chat to see Airpost.');
      }
      result = {
        ok: run.ok,
        message,
        connected: run.ok ? action === 'install' : null,
        canForce: !run.ok && action === 'install' && message.includes('--force'),
      };
    } catch (err) {
      result = { ok: false, message: (err as Error).message, connected: null, canForce: false };
    }
    if (!this._mounted) return;
    this.setState({ busyClient: null, results: { ...this.state.results, [client]: result } });
  };

  _renderTrayNote() {
    if (!this.state.trayTurnedOn) return null;
    return (
      <SettingsNote>
        {process.platform === 'darwin'
          ? localized('Airpost now shows in the menu bar, so you can see that it is running.')
          : localized(
              'Airpost now keeps running in the system tray after you close the window, so apps keep working.'
            )}{' '}
        <a onClick={() => Actions.switchPreferencesTab('General')}>
          {localized('Change in General')}
        </a>
      </SettingsNote>
    );
  }

  _lastUsed(client: McpClientId) {
    return this.state.audit.find((e) => clientForEntry(e)?.id === client);
  }

  _renderClient({ id, name }: { id: McpClientId; name: string }) {
    const { busyClient } = this.state;
    const result = this.state.results[id];
    const available = !!airpostMcpBinary();
    const lastUsed = this._lastUsed(id);
    const connected = result?.connected ?? null;

    let subtitle: React.ReactNode = lastUsed
      ? localized('Used %@', formatTime(lastUsed.timestamp))
      : localized('Not used yet');
    if (result) {
      subtitle = (
        <span className={`agents-client-result ${result.ok ? 'ok' : 'failed'}`}>
          {result.message}
        </span>
      );
    }

    const disabled = !available || !!busyClient;
    return (
      <ProviderCard
        key={id}
        logo={<ProviderLogo {...CLIENT_LOGOS[id]} />}
        title={name}
        subtitle={subtitle}
        status={
          connected !== null && (
            <StatusPill kind={connected ? 'ok' : 'off'}>
              {connected ? localized('Connected') : localized('Not connected')}
            </StatusPill>
          )
        }
        actions={
          <>
            {result?.canForce && (
              <Button
                variant="outline"
                disabled={!!busyClient}
                onClick={() => this._onRunClient(id, 'install', true)}
              >
                {localized('Replace It')}
              </Button>
            )}
            {connected !== true && (
              <Button
                variant="outline"
                disabled={disabled}
                onClick={() => this._onRunClient(id, 'install')}
              >
                {localized('Connect')}
              </Button>
            )}
            {connected !== false && (
              <Button
                variant="outline"
                disabled={disabled}
                onClick={() => this._onRunClient(id, 'uninstall')}
              >
                {localized('Disconnect')}
              </Button>
            )}
          </>
        }
      />
    );
  }

  _renderLocalApps() {
    const { policyError } = this.state;
    const on = this._isOn();
    const label = localized('Allow apps over stdio');
    return (
      <SettingsSection
        title={localized('Local apps (stdio)')}
        note={
          airpostMcpBinary()
            ? localized(
                'Connecting adds Airpost to the app’s own settings. Airpost can’t tell whether an app is still connected after this window closes.'
              )
            : localized(
                'This copy of Airpost can’t connect apps because it doesn’t include airpost-mcp. Install Airpost from the official download to use this.'
              )
        }
      >
        <SettingsRow
          label={label}
          description={localized(
            'Claude Desktop, Claude Code, Codex and other MCP apps on this computer start Airpost’s MCP server (airpost-mcp) themselves and talk to it over stdio.'
          )}
        >
          <SettingsSwitch
            checked={on && !policyError}
            disabled={!!policyError}
            label={label}
            onChange={this._onToggleAccess}
          />
        </SettingsRow>
        <Dimmed off={!on} className="agents-clients">
          {MCP_CLIENTS.map((c) => this._renderClient(c))}
        </Dimmed>
      </SettingsSection>
    );
  }

  _httpServerText() {
    const { http, httpError } = this.state;
    if (httpError) return localized('airpost couldn’t run it: %@', httpError);
    if (!http || !http.installed) return localized('Not installed.');
    if (http.running) return localized('Serving %@.', http.url);
    return localized('Installed, but not running.');
  }

  _renderServerControl() {
    const { http, httpBusy, policy } = this.state;
    if (http?.installed && http.running) return null;
    const action = http?.installed ? 'start' : 'install';
    return (
      <Button
        variant="outline"
        disabled={httpBusy}
        onClick={async () => {
          this.setState({ httpBusy: true, httpError: null });
          const result = await runServiceCommand('mcp-http', action, {
            port: policy.http.port,
            run: this.props.run,
          });
          if (!this._mounted) return;
          this.setState({ httpBusy: false });
          await this._refreshHttp();
          if (this._mounted && !result.ok) this.setState({ httpError: result.error });
        }}
      >
        {action === 'start' ? localized('Start') : localized('Install')}
      </Button>
    );
  }

  _renderHttp() {
    const { policy, policyError, portInput, http, httpBusy } = this.state;
    const label = localized('Allow apps over HTTP');
    const tokenFile = path.join(configDir(), TOKEN_FILE);
    const pill = http?.installed ? servicePill(http) : null;
    return (
      <SettingsSection title={localized('HTTP')}>
        <SettingsRow
          label={label}
          description={localized(
            'For apps that can’t start Airpost themselves. Airpost installs the server (airpost-mcp http) as a background service, and only programs on this computer can connect.'
          )}
        >
          <SettingsSwitch
            checked={policy.http.enabled && !policyError}
            disabled={!!policyError || httpBusy}
            label={label}
            onChange={this._onToggleHttp}
          />
        </SettingsRow>
        {policy.http.enabled && !policyError && (
          <>
            <SettingsRow
              label={localized('Port')}
              description={localized('Apps connect to http://127.0.0.1:%@/mcp.', policy.http.port)}
            >
              <TextField
                id="mcp-http-port"
                className="agents-port"
                type="text"
                inputMode="numeric"
                aria-label={localized('Port')}
                value={portInput}
                onChange={(e) => this.setState({ portInput: e.target.value })}
                onBlur={this._onCommitPort}
                onKeyDown={(e) => e.key === 'Enter' && this._onCommitPort()}
              />
            </SettingsRow>
            <SettingsRow
              label={localized('Access key')}
              description={localized(
                'Apps send it as “Authorization: Bearer <key>”. It is kept in %@.',
                tokenFile
              )}
            >
              <Button
                variant="outline"
                disabled={!fs.existsSync(tokenFile)}
                onClick={this._onCopyKey}
              >
                {localized('Copy Key')}
              </Button>
            </SettingsRow>
            <SettingsRow label={localized('Server')} description={this._httpServerText()}>
              {pill && <StatusPill kind={pill.kind}>{pill.text}</StatusPill>}
              {this._renderServerControl()}
            </SettingsRow>
          </>
        )}
      </SettingsSection>
    );
  }

  _renderAccessLevel() {
    const { policy } = this.state;
    const known = ACCESS_LEVELS.includes(policy.accessLevel as AccessLevel);
    const level = (known ? policy.accessLevel : 'read-only') as AccessLevel;
    return (
      <SettingsSection title={localized('What apps can do')}>
        <SettingsRow
          label={localized('Access')}
          description={
            localized(ACCESS_LEVEL_DESCRIPTIONS[level]) +
            ' ' +
            localized('Every connected app gets the same access.')
          }
        >
          <SettingsSelect
            id="mcp-access-level"
            aria-label={localized('Access')}
            value={policy.accessLevel}
            onChange={this._onAccessLevel}
            options={[
              ...(known ? [] : [[policy.accessLevel, localized('Read mail')] as [string, string]]),
              ...ACCESS_LEVELS.map((l): [string, string] => [l, localized(ACCESS_LEVEL_LABELS[l])]),
            ]}
          />
        </SettingsRow>
      </SettingsSection>
    );
  }

  _renderAccount(account: Account) {
    const enabled = this._isAccountEnabled(account.id);
    const excluded = this.state.policy.enabledAccounts[account.id]?.excludedFolderIds || [];
    const expanded = enabled && !!this.state.expanded[account.id];
    const name = account.label || account.emailAddress;
    let description: string | null = null;
    if (enabled) {
      description = excluded.length
        ? localized('%@ folders hidden', excluded.length)
        : localized('Every folder');
    }
    return (
      <React.Fragment key={account.id}>
        <SettingsRow label={name} description={description}>
          {enabled && (
            <Button
              variant="outline"
              onClick={() =>
                this.setState({ expanded: { ...this.state.expanded, [account.id]: !expanded } })
              }
            >
              {expanded ? localized('Done') : localized('Choose folders')}
            </Button>
          )}
          <SettingsSwitch
            checked={enabled}
            label={name}
            onChange={() => this._onToggleAccount(account.id)}
          />
        </SettingsRow>
        {expanded && (
          <div className="agents-folders">
            {CategoryStore.categories(account.id).map((cat) => (
              <label key={cat.id} className="agents-folder">
                <Checkbox
                  checked={!excluded.includes(cat.id)}
                  onCheckedChange={() => this._onToggleFolder(account.id, cat.id)}
                />
                {cat.displayName || cat.name}
              </label>
            ))}
          </div>
        )}
      </React.Fragment>
    );
  }

  _renderAccounts() {
    const accounts = AccountStore.accounts();
    return (
      <SettingsSection title={localized('Mail apps can see')}>
        {accounts.length === 0 ? (
          <SettingsRow label={localized('No accounts yet')} />
        ) : (
          accounts.map((a) => this._renderAccount(a))
        )}
      </SettingsSection>
    );
  }

  _renderActivityEntry(entry: AuditEntry, i: number) {
    const client = clientForEntry(entry);
    const who = client ? client.name : (entry.client || localized('An app')).split(' ')[0];
    const what = TOOL_PHRASES[entry.toolName]
      ? localized(TOOL_PHRASES[entry.toolName])
      : localized('used %@', entry.toolName);
    const outcome = entry.outcome || 'ok';
    let detail: React.ReactNode = paramsText(entry.params);
    if (outcome === 'denied') detail = localized('Blocked: %@', blockedText(entry));
    if (outcome === 'error') detail = localized('Failed: %@', errorText(entry));
    let summary = `${who} ${what}`;
    if (outcome === 'denied') summary = localized('%@ was blocked', who);
    if (outcome === 'error') summary = localized('%@ ran into a problem', who);
    const logo = client ? CLIENT_LOGOS[client.id] : { text: who.slice(0, 1), color: '#5d6472' };
    return (
      <div key={i} className={`agents-activity outcome-${outcome}`}>
        {outcome === 'ok' ? (
          <span className="agents-activity-dot" style={{ background: logo.color }} aria-hidden>
            {logo.text}
          </span>
        ) : (
          <span className="agents-activity-dot bad" aria-hidden>
            !
          </span>
        )}
        <div className="agents-activity-text">
          <b title={entry.toolName}>{summary}</b>
          {detail && (
            <small className="agents-activity-detail" title={entry.params}>
              {detail}
            </small>
          )}
        </div>
        <time title={new Date(entry.timestamp).toLocaleString()}>
          {formatTime(entry.timestamp)}
        </time>
      </div>
    );
  }

  _renderActivity() {
    const { audit, showAllActivity } = this.state;
    const shown = showAllActivity ? audit : audit.slice(0, ACTIVITY_SHOWN);
    return (
      <SettingsSection
        title={localized('Activity')}
        aside={
          <a className="agents-refresh" onClick={this._reloadAudit}>
            {localized('Refresh')}
          </a>
        }
        note={
          audit.length > ACTIVITY_SHOWN && (
            <a onClick={() => this.setState({ showAllActivity: !showAllActivity })}>
              {showAllActivity ? localized('Show less') : localized('Show all activity')}
            </a>
          )
        }
      >
        {audit.length === 0 ? (
          <SettingsRow
            label={localized('Nothing yet')}
            description={localized('What apps do with your mail shows up here.')}
          />
        ) : (
          shown.map((e, i) => this._renderActivityEntry(e, i))
        )}
      </SettingsSection>
    );
  }

  _renderStatusRow() {
    const { status, statusError } = this.state;
    if (!airpostMcpBinary()) {
      return (
        <SettingsRow
          label={localized('Connection to your mail')}
          description={localized(
            'airpost-mcp is not in this build of Airpost. Build it with `cargo build --release -p airpost-mcp` and set AIRPOST_MCP_BIN.'
          )}
        />
      );
    }
    let text = localized('Checking…');
    if (statusError) text = statusError;
    if (status) {
      const accounts = status.engines.reduce((n, e) => n + e.accounts.length, 0);
      if (status.engines.length === 0) {
        text = localized("Apps can't reach your mail right now. They can while Airpost is open.");
      } else if (status.engines.length === 1) {
        text = accounts
          ? localized('Apps can reach %@ accounts.', accounts)
          : localized('Airpost is running, with no accounts apps can reach yet.');
      } else {
        text = localized(
          '%1$@ copies of Airpost are running, with %2$@ accounts.',
          status.engines.length,
          accounts
        );
      }
      if (status.policy?.error) text += ` ${status.policy.error}`;
    }
    return (
      <SettingsRow label={localized('Connection to your mail')} description={text}>
        <Button variant="outline" onClick={this._refreshStatus}>
          {localized('Check again')}
        </Button>
      </SettingsRow>
    );
  }

  render() {
    const { policyError, policyErrorFile, policy } = this.state;
    const off = !this._isOn() && !policy.http.enabled;
    return (
      <div className="agents-section">
        {policyError && (
          <SettingsSection danger>
            <SettingsRow
              label={localized('Airpost can’t read the settings for apps')}
              description={localized(
                '%1$@ is not valid (%2$@). Fix or delete it.',
                policyErrorFile || policyPath(configDir()),
                policyError
              )}
            />
          </SettingsSection>
        )}
        {this._renderLocalApps()}
        {this._renderTrayNote()}
        {this._renderHttp()}
        <Dimmed off={off}>
          {!policyError && this._renderAccessLevel()}
          {!policyError && this._renderAccounts()}
          {this._renderActivity()}
        </Dimmed>
        <SettingsSection title={localized('Status')}>{this._renderStatusRow()}</SettingsSection>
      </div>
    );
  }
}
