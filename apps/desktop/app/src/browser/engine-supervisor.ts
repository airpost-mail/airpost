// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Runs the Rust engine's `serve` mode from the main process (docs/desktop-client-plan.md 4.1
step 1), from app launch and with zero accounts, so a window reload leaves it running and
first-run onboarding has an engine to test settings with. The supervisor is the one owner of
serve's stdin: it adds and removes the accounts in config.json `accounts` with their secrets
from KeyManager (in main too), stores the rotated tokens serve reports on stdout, applies
EngineClient's relaunch rules, and makes the stdin-only calls windows ask for over IPC
(account.test, account.reset, engine.resetDatabase, engine.setLogLevel).

Windows open their own socket connections (src/engine-connection.ts) once Status names a
ready engine. Its own connection subscribes to no model class, which leaves the deltas and the
first subscriber's backlog to the windows and still brings it every `account.state` and
`settings.changed`; it also carries the engine-owned settings (src/browser/engine-settings.ts). Account
events (launched, exited, too many failures) go to the main window, which updates AccountStore
and OnlineStatusStore; it acks them, and those not acked are handed to its next renderer.

An engine with Feature.Accounts keeps the accounts and their secrets itself (section 5 of the
plan). Before the first account.add to it, the supervisor sends `credentials.import` with every
config.json account the engine hasn't registered, and with every account KeyManager still has
secrets for (new ones from onboarding or a reconnect, or from a downgraded build); once the
engine has them all, KeyManager drops the accounts' secrets (moveAccountSecretsToEngine) and
account.add goes without them. After that, an account.add whose secrets the engine stored drops
that account's copy too: the engine keeps its rotated tokens, so the copy would go stale.
config.json `accounts` stays the list of accounts the desktop runs; the accounts deleted from it
are kept under accountsToRemoveFromEngine until the engine's account.remove answers, and the
engine's other accounts are left alone (AccountStore lists them). A `serve --detached` engine found at launch with the bundled build is joined instead
of spawned (detachedEngineToReuse); it can't take `credentials.import`, so it is reused only
once the credentials moved. An installed engine unit running another build is restarted and
then joined (engine-unit.ts).
*/
import fs from 'fs';
import path from 'path';
import {
  AccountJSON,
  CONFIG_ACCOUNTS_TO_REMOVE_KEY,
  EngineAccount,
  EngineClient,
} from '../engine-client';
import net from 'net';
import {
  engineBinaryPath,
  engineBuildId,
  EngineServeProcess,
  MailsyncProcessExit,
  ResetOptions,
} from '../mailsync-process';
import { AccountSecretsJSON, isAccountSecretKey, KeyManager, KeySet } from './key-manager';
import {
  CredentialsImportParams,
  Feature,
  HelloResult,
  Method,
  PROTOCOL_VERSION,
  SettingsValues,
} from '../engine-protocol';
import type { EngineSettingsMirror } from './engine-settings';
import {
  AccountEvent,
  EngineAccountRoute,
  EngineStatus,
  SupervisorChannel,
} from '../supervisor-channels';

const VERBOSE_UNTIL_KEY = 'core.sync.verboseUntil';
const SECRETS_UPDATED = 'ProcessAccountSecretsUpdated';

// Account events kept for a main window that hasn't acked them. The oldest go first: the main
// window only misses events while it has no renderer.
const MAX_PENDING_EVENTS = 500;

export interface SupervisorConfig {
  get(keyPath: string): any;
  set(keyPath: string, value: unknown): void;
  onDidChange(keyPath: string, callback: (event: unknown) => void): { dispose(): void };
}

// A window's renderer, as the supervisor talks to it (a WebContents in the app).
export interface SupervisorTarget {
  send(channel: string, ...args: any[]): void;
}

export interface EngineSupervisorOptions {
  configDirPath: string;
  resourcePath: string;
  config: SupervisorConfig;
  keyManager: KeyManager;
  // Sends Status to every window.
  broadcast: (channel: string, payload: unknown) => void;
  // Gets each engine once its socket is greeted, and the settings it announces as changed.
  settings?: Pick<EngineSettingsMirror, 'attach' | 'detach' | 'engineChanged'>;
  // The socket of a detached engine to join instead of spawning the first one.
  attachTo?: string | null;
}

type PendingEvent = AccountEvent & { seq: number };

export class EngineSupervisor {
  options: EngineSupervisorOptions;
  client: EngineClient;
  stopped = false;

  // The engine whose socket windows should use, and its number.
  _readyEngine: EngineServeProcess = null;
  _generation = 0;
  _readyGeneration: number = null;

  _mainWindow: SupervisorTarget = null;
  _pending: PendingEvent[] = [];
  _nextSeq = 1;

  _ensureQueued = false;
  _statusQueued = false;
  _configSubscriptions: Array<{ dispose(): void }> = [];
  _attachTo: string | null = null;
  // The account ids config.json listed at the last change, to tell which ones were deleted.
  _listedIds = new Set<string>();
  // Settles once the engine's hello was handled by _prepareCredentials, or it closed.
  _credentialsReady = new WeakMap<EngineServeProcess, Promise<void>>();

  constructor(options: EngineSupervisorOptions) {
    this.options = options;
    this._attachTo = options.attachTo || null;
    this.client = new EngineClient({
      createEngine: () => this._createEngine(),
      accounts: () => this.accounts(),
      accountJSONs: (accounts) => this._accountJSONs(accounts),
      onDeltas: (msgs) => this._onParentDeltas(msgs),
      onAccountExited: (accountId) => this._emit({ type: 'exited', accountId }),
      onAccountLaunched: (json) =>
        this._emit({ type: 'launched', accountId: json.id, syncState: json.syncState }),
      onTooManyFailures: (accountId, exit, authFailure) =>
        this._emit({
          type: 'too-many-failures',
          accountId,
          authFailure,
          exit: serializeExit(exit),
        }),
      onAccountsChanged: () => this._statusChanged(),
      onAccountStored: (accountId) => this._dropStoredSecrets(accountId),
      removedAccounts: () => this.removedAccounts(),
      onAccountRemoved: (accountId) => this._forgetRemoved(accountId),
    });
  }

  start() {
    const { config } = this.options;
    this._listedIds = new Set(this.accounts().map((a) => a.id));
    this._configSubscriptions = [
      config.onDidChange('accounts', () => {
        this._noteRemovedAccounts();
        this.ensure();
      }),
      config.onDidChange(VERBOSE_UNTIL_KEY, () => this._applyLogLevel()),
    ];
    this.ensure();
  }

  // Coalesces the config changes of one AccountStore save into one diff.
  ensure() {
    if (this._ensureQueued || this.stopped) return;
    this._ensureQueued = true;
    Promise.resolve().then(() => {
      this._ensureQueued = false;
      if (!this.stopped) this.client.ensure();
    });
  }

  // The accounts config.json lists, skipping the ones AccountStore refuses to load.
  accounts(): Array<AccountSecretsJSON & { id: string }> {
    const seenIds = new Set<string>();
    const seenEmails = new Set<string>();
    const result: Array<AccountSecretsJSON & { id: string }> = [];
    for (const json of this.options.config.get('accounts') || []) {
      if (!json || !json.id || !json.emailAddress) continue;
      if (seenIds.has(json.id) || seenEmails.has(json.emailAddress)) continue;
      seenIds.add(json.id);
      seenEmails.add(json.emailAddress);
      result.push(json);
    }
    return result;
  }

  // Accounts deleted in the app that the engine may still have registered.
  removedAccounts(): string[] {
    const listed = new Set(this.accounts().map((a) => a.id));
    return this._removedList().filter((id) => !listed.has(id));
  }

  _removedList(): string[] {
    const list = this.options.config.get(CONFIG_ACCOUNTS_TO_REMOVE_KEY);
    return Array.isArray(list) ? list.filter((id) => typeof id === 'string' && id) : [];
  }

  // Kept in config.json, so a delete made while no engine runs reaches the next one.
  _noteRemovedAccounts() {
    const now = new Set(this.accounts().map((a) => a.id));
    const gone = [...this._listedIds].filter((id) => !now.has(id));
    this._listedIds = now;
    const before = this._removedList();
    const next = [...new Set([...before, ...gone])].filter((id) => !now.has(id));
    if (next.length !== before.length || next.some((id, i) => id !== before[i])) {
      this.options.config.set(CONFIG_ACCOUNTS_TO_REMOVE_KEY, next);
    }
  }

  _forgetRemoved(accountId: string) {
    const before = this._removedList();
    if (!before.includes(accountId)) return;
    this.options.config.set(
      CONFIG_ACCOUNTS_TO_REMOVE_KEY,
      before.filter((id) => id !== accountId)
    );
  }

  status(): EngineStatus {
    const accounts: { [id: string]: EngineAccountRoute } = {};
    for (const [id, entry] of this.client.entries) {
      accounts[id] = { state: entry.state, held: this.client.held.has(id) };
    }
    for (const id of this.client.held.keys()) {
      if (!accounts[id]) accounts[id] = { state: null, held: true };
    }
    const ready = this._readyEngine !== null && this._readyEngine === this.client.engine;
    return {
      generation: ready ? this._readyGeneration : null,
      socketPath: ready ? this._readyEngine._socketPath : null,
      accounts,
    };
  }

  // The main window's renderer started (or restarted): it gets the events it hasn't acked,
  // and every later one as it happens.
  attachMainWindow(target: SupervisorTarget): AccountEvent[] {
    this._mainWindow = target;
    return this._pending.slice();
  }

  detachMainWindow(target: SupervisorTarget) {
    if (this._mainWindow === target) this._mainWindow = null;
  }

  ackAccountEvents(seq: number) {
    this._pending = this._pending.filter((e) => e.seq > seq);
  }

  forceRelaunch(accountId: string) {
    return this.client.forceRelaunch({ id: accountId });
  }

  // The account may already be gone from config.json, as when it is being deleted.
  resetCache(accountId: string, options: ResetOptions = {}) {
    return this.client.resetCache(accountId, () =>
      this.client.withEngine((engine) => engine.resetAccount(accountId, options))
    );
  }

  resetDatabase(options: ResetOptions = {}) {
    return this.client.resetDatabase(options);
  }

  shutdown() {
    this.stopped = true;
    for (const subscription of this._configSubscriptions) subscription.dispose();
    this._readyEngine = null;
    this._statusChanged();
    return this.client.shutdown();
  }

  // Private

  _verboseUntil(): number {
    return Number(this.options.config.get(VERBOSE_UNTIL_KEY)) || 0;
  }

  _processOptions() {
    const { configDirPath, resourcePath } = this.options;
    const verboseUntil = this._verboseUntil();
    const verbose = verboseUntil > Date.now();
    if (verbose) {
      console.warn(`Verbose mailsync logging is enabled until ${new Date(verboseUntil)}`);
    }
    return { configDirPath, resourcePath, verbose };
  }

  // A running engine restarts its accounts with the new level; a later one starts with it.
  _applyLogLevel() {
    const { engine } = this.client;
    if (!engine) return;
    engine.setLogLevel(this._verboseUntil()).catch((err) => {
      if (!err.engineExited) console.warn(`engine.setLogLevel failed: ${err.message}`);
    });
  }

  _createEngine() {
    const engine = new EngineServeProcess(this._processOptions());
    engine.subscription = { accounts: null, classes: [] };
    // Only the first engine joins a detached one; after that the supervisor spawns its own.
    engine.attachTo = this._attachTo;
    this._attachTo = null;
    const generation = ++this._generation;
    let credentialsSettled: () => void;
    this._credentialsReady.set(engine, new Promise((resolve) => (credentialsSettled = resolve)));
    engine.once('hello', (hello: HelloResult) => {
      this._prepareCredentials(engine, hello)
        .catch((err) => console.error(`Could not move the account secrets to the engine: ${err}`))
        .then(() => credentialsSettled());
    });
    engine.once('close', () => credentialsSettled());
    engine.on('ready', () => {
      if (this.client.engine !== engine) return;
      this._readyEngine = engine;
      this._readyGeneration = generation;
      this._statusChanged();
    });
    const { settings } = this.options;
    engine.on('close', () => {
      if (this._readyEngine === engine) this._readyEngine = null;
      if (settings) settings.detach(engine);
      this._statusChanged();
    });
    if (settings) {
      engine.on('hello', () => {
        if (this.client.engine === engine) settings.attach(engine);
      });
      engine.on('settings-changed', (values: SettingsValues) =>
        settings.engineChanged(engine, values)
      );
    }
    return engine;
  }

  // The accounts as account.add takes them. An engine that keeps the secrets gets those
  // KeyManager still has (an account added or reconnected this run) and fills in the rest.
  async _accountJSONs(accounts: EngineAccount[]): Promise<AccountJSON[]> {
    const { keyManager } = this.options;
    const { engine } = this.client;
    const ready = engine && this._credentialsReady.get(engine);
    if (ready) await ready;
    const registry = this.client.registry();
    const byId = new Map(this.accounts().map((json) => [json.id, json]));
    const keys = await keyManager._getKeyHash();
    const result: AccountJSON[] = [];
    for (const { id } of accounts) {
      const json = byId.get(id);
      if (!json) continue;
      if (registry && !(await keyManager.hasAccountSecrets(json.emailAddress, keys))) {
        result.push(json as unknown as AccountJSON);
      } else {
        result.push((await keyManager.insertAccountSecrets(json, keys)) as unknown as AccountJSON);
      }
    }
    return result;
  }

  // The engine stored the secrets account.add carried (a new or signed-in-again account), and
  // keeps the rotated tokens from now on, so KeyManager's copy would only go stale. Only once
  // the move is done: before it, credentialsBeforeEngine doesn't exist yet, and the copy is
  // what a downgrade and the windows (reconnect, OAuth revocation) read.
  _dropStoredSecrets(accountId: string) {
    const json = this.accounts().find((a) => a.id === accountId);
    const { keyManager } = this.options;
    if (!json || !keyManager.credentialsMovedToEngine()) return;
    keyManager
      .hasAccountSecrets(json.emailAddress)
      .then((has) => has && keyManager.deleteAccountSecrets(json))
      .catch((err) => console.warn(`Could not drop the secrets the engine stored: ${err}`));
  }

  /*
  Migration steps 2-5 (plan section 5): imports the accounts the engine lacks, with the secrets
  KeyManager has for them, falling back to the keyset kept from before the move (a downgraded
  build may have added accounts, and re-signed-in ones have newer secrets in `credentials`).
  Once every account is in the engine, KeyManager drops the accounts' secrets.
  */
  async _prepareCredentials(engine: EngineServeProcess, hello: HelloResult) {
    if (engine._attached || !(hello.features || []).includes(Feature.Accounts)) return;
    const { keyManager } = this.options;
    const keys = await keyManager._getKeyHash();
    const moved = keyManager.credentialsMovedToEngine();
    const registered = new Set((hello.accounts || []).map((a) => a.accountId));
    let earlier: KeySet = null;
    const accounts: CredentialsImportParams['accounts'] = [];
    for (const json of this.accounts()) {
      const own = await keyManager.hasAccountSecrets(json.emailAddress, keys);
      if (registered.has(json.id) && !own) continue;
      if (!own && !earlier) earlier = await keyManager.keysBeforeEngine();
      // An account with no secrets anywhere needs a reconnect, moved or not; the engine would
      // refuse it and keep the move from ever completing.
      if (!own && !(await keyManager.hasAccountSecrets(json.emailAddress, earlier))) {
        console.warn(
          `Account ${json.id} has no secrets to move to the engine; it needs a reconnect`
        );
        continue;
      }
      accounts.push(await keyManager.insertAccountSecrets(json, own ? keys : earlier));
    }
    const secrets: CredentialsImportParams['secrets'] = {};
    if (!moved) {
      for (const [name, value] of Object.entries(keys)) {
        if (!isAccountSecretKey(name) && typeof value === 'string' && value) secrets[name] = value;
      }
    }
    const leftover = Object.keys(keys).some((name) => isAccountSecretKey(name) && keys[name]);
    if (!accounts.length && !Object.keys(secrets).length && moved && !leftover) return;
    if (accounts.length || Object.keys(secrets).length) {
      const result = await engine.importCredentials({ accounts, secrets });
      for (const { accountId, email, error } of result.failed || []) {
        console.warn(`The engine did not take account ${accountId} (${email}): ${error}`);
      }
      if ((result.failed || []).length) return;
    }
    await keyManager.moveAccountSecretsToEngine();
  }

  // serve's stdout carries only the accounts' rotated secrets.
  _onParentDeltas(msgs: string[]) {
    for (const msg of msgs) {
      let json = null;
      try {
        json = JSON.parse(msg);
      } catch (err) {
        console.warn('The sync engine wrote a delta that is not JSON on stdout');
        continue;
      }
      if (json.modelClass !== SECRETS_UPDATED || !json.modelJSONs || !json.modelJSONs.length) {
        console.warn(`Ignored a ${json.modelClass} delta on the sync engine's stdout`);
        continue;
      }
      this.options.keyManager.extractAndStoreAccountSecrets(json.modelJSONs[0]).catch((err) => {
        console.error(`Could not store the account secrets the sync engine updated: ${err}`);
      });
    }
  }

  _emit(event: AccountEvent) {
    const pending = { ...event, seq: this._nextSeq++ } as PendingEvent;
    this._pending.push(pending);
    if (this._pending.length > MAX_PENDING_EVENTS) this._pending.shift();
    if (this._mainWindow) {
      try {
        this._mainWindow.send(SupervisorChannel.AccountEvent, pending);
      } catch (err) {
        // A destroyed window; the next one attaches and gets it.
        this._mainWindow = null;
      }
    }
  }

  // Several changes in one turn (an engine exit drops every account) send one Status.
  _statusChanged() {
    if (this._statusQueued) return;
    this._statusQueued = true;
    Promise.resolve().then(() => {
      this._statusQueued = false;
      this.options.broadcast(SupervisorChannel.Status, this.status());
    });
  }
}

function serializeExit(exit: MailsyncProcessExit) {
  const error = exit.error as Error & { syncError?: unknown };
  return {
    code: exit.code === undefined ? null : exit.code,
    signal: exit.signal || null,
    error: error ? { message: error.message, syncError: error.syncError } : null,
  };
}

// Errors cross IPC as text.
async function reply(work: () => Promise<unknown>) {
  try {
    await work();
    return {};
  } catch (err) {
    return { error: (err && err.message) || `${err}` };
  }
}

/*
Wires the supervisor to the windows. `supervisor` is null in spec mode, where no engine runs:
Status is then null.
*/
export function registerEngineSupervisorIPC(
  ipcMain: Electron.IpcMain,
  supervisor: EngineSupervisor | null
) {
  ipcMain.handle(SupervisorChannel.GetStatus, () => (supervisor ? supervisor.status() : null));
  if (!supervisor) return;

  // A reload keeps the WebContents, so it attaches again without a new listener.
  const watched = new WeakSet<Electron.WebContents>();
  ipcMain.handle(SupervisorChannel.AttachMainWindow, (event) => {
    const contents = event.sender;
    if (!watched.has(contents)) {
      watched.add(contents);
      contents.once('destroyed', () => supervisor.detachMainWindow(contents));
    }
    return supervisor.attachMainWindow(contents);
  });
  ipcMain.on(SupervisorChannel.AckAccountEvents, (_event, seq: number) =>
    supervisor.ackAccountEvents(seq)
  );
  ipcMain.handle(SupervisorChannel.ForceRelaunch, (_event, accountId: string) =>
    supervisor.forceRelaunch(accountId)
  );
  ipcMain.handle(
    SupervisorChannel.ResetCache,
    (_event, accountId: string, options?: ResetOptions) =>
      reply(() => supervisor.resetCache(accountId, options))
  );
  ipcMain.handle(SupervisorChannel.ResetDatabase, (_event, options?: ResetOptions) =>
    reply(() => supervisor.resetDatabase(options))
  );
}

// How long launch waits for an engine from an earlier run before migrating anyway: serve exits
// 30 s after its stdin closed, and an orphan may still be stopping its accounts after that.
export const EARLIER_ENGINE_WAIT_MS = 40000;
const EARLIER_ENGINE_POLL_MS = 250;

export function pidAlive(pid: number) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    // EPERM: it exists but belongs to someone else.
    return err.code === 'EPERM';
  }
}

// The pid in run/engine.json, which serve writes once bound and removes as it exits.
export function earlierEnginePid(configDirPath: string): number | null {
  try {
    const file = path.join(configDirPath, 'run', 'engine.json');
    const { pid } = JSON.parse(fs.readFileSync(file).toString());
    return Number.isInteger(pid) && pid > 0 && pid !== process.pid ? pid : null;
  } catch (err) {
    return null;
  }
}

/*
Resolves once the `serve` engine of an earlier run (a relaunch while it shuts down, an orphan
of a crashed app) has exited, so the launch's `migrate` never runs beside an engine that still
has the database open. Resolves false when it was still running after `timeoutMs`.
*/
export async function waitForEarlierEngine(
  configDirPath: string,
  { timeoutMs = EARLIER_ENGINE_WAIT_MS, pollMs = EARLIER_ENGINE_POLL_MS } = {}
) {
  const pid = earlierEnginePid(configDirPath);
  if (pid === null || !pidAlive(pid)) return true;
  console.log(`Waiting for the sync engine of an earlier run (pid ${pid}) to exit`);
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, pollMs));
    if (!pidAlive(pid)) return true;
  }
  console.warn(`The sync engine of an earlier run (pid ${pid}) is still running; migrating anyway`);
  return false;
}

interface EngineDiscovery {
  pid: number;
  socket: string;
  protocol?: number;
  detached?: boolean;
}

function readEngineDiscovery(configDirPath: string): EngineDiscovery | null {
  try {
    const file = path.join(configDirPath, 'run', 'engine.json');
    const json = JSON.parse(fs.readFileSync(file).toString());
    const { pid, socket } = json;
    if (!Number.isInteger(pid) || pid <= 0 || pid === process.pid) return null;
    if (typeof socket !== 'string' || !socket) return null;
    return json as EngineDiscovery;
  } catch (err) {
    return null;
  }
}

const PROBE_TIMEOUT_MS = 5000;

/*
Opens a connection to an engine socket, sends `calls` in order after a hello and resolves with
each answer (the hello's first) once all have come, or rejects when the connection ends or
`timeoutMs` passes first.
*/
export function engineSocketExchange(
  socketPath: string,
  calls: Array<[string, unknown]>,
  timeoutMs = PROBE_TIMEOUT_MS
): Promise<Array<{ result?: any; error?: { code: number; message: string } }>> {
  return new Promise((resolve, reject) => {
    const all: Array<[string, unknown]> = [
      [Method.Hello, { protocol: PROTOCOL_VERSION, client: 'tern-desktop/launch' }],
      ...calls,
    ];
    const answers = new Array(all.length);
    let left = all.length;
    let buffer = '';
    const socket = net.createConnection(socketPath);
    const done = (err: Error | null) => {
      clearTimeout(timer);
      socket.removeAllListeners();
      socket.on('error', () => {});
      socket.destroy();
      if (err) reject(err);
      else resolve(answers);
    };
    const timer = setTimeout(() => done(new Error('the engine did not answer')), timeoutMs);
    socket.setEncoding('utf8');
    socket.on('data', (chunk: string) => {
      buffer += chunk;
      const lines = buffer.split('\n');
      buffer = lines.pop();
      for (const line of lines) {
        let msg = null;
        try {
          msg = JSON.parse(line);
        } catch (err) {
          continue;
        }
        if (msg.method !== undefined || !Number.isInteger(msg.id)) continue;
        if (msg.id < 1 || msg.id > all.length || answers[msg.id - 1]) continue;
        answers[msg.id - 1] = { result: msg.result, error: msg.error };
        if (--left === 0) {
          done(null);
          return;
        }
      }
    });
    socket.on('error', (err) => done(err));
    socket.on('close', () => done(new Error('the engine closed the connection')));
    socket.write(
      all
        .map(([method, params], i) => JSON.stringify({ jsonrpc: '2.0', id: i + 1, method, params }))
        .join('\n') + '\n'
    );
  });
}

/*
The socket of a `serve --detached` engine (run/engine.json) the launch can join instead of
spawning its own: one that runs the bundled build and protocol, keeps the accounts, and only
once `canReuse` (the credentials moved into the engine; a joined engine can't take
`credentials.import`). Null when there is none to join.

With an engine unit installed (`restartUnit`, engine-unit.ts), an engine that is only of
another build or protocol (the unit's binary path holds this build after an update) is
restarted through the unit and the restarted one joined. Any other detached engine, and one
still not joinable after the restart, is asked to stop (engine.shutdown on its socket), so
waitForEarlierEngine then waits for it to exit and the bundled one starts; the app starts the
unit again when it quits. An earlier engine that isn't detached is left to
waitForEarlierEngine: it exits once its parent's stdin closed.
*/
export async function detachedEngineToReuse(
  configDirPath: string,
  resourcePath: string,
  {
    canReuse,
    timeoutMs = PROBE_TIMEOUT_MS,
    restartUnit,
    restartWaitMs = UNIT_RESTART_WAIT_MS,
  }: {
    canReuse: boolean;
    timeoutMs?: number;
    restartUnit?: () => Promise<boolean>;
    restartWaitMs?: number;
  }
): Promise<string | null> {
  let found = readEngineDiscovery(configDirPath);
  if (!found || !found.detached || !pidAlive(found.pid)) return null;
  const bundled = await engineBuildId(engineBinaryPath(resourcePath));
  let reason = await reasonNotToJoin(found, bundled, canReuse, timeoutMs);
  if (reason === UNREACHABLE) return null;
  if (reason && canReuse && restartUnit) {
    console.log(`Restarting the engine unit (pid ${found.pid}): ${reason}`);
    if (!(await restartUnit())) {
      console.warn('Could not restart the engine unit; stopping its engine instead');
    } else {
      const restarted = await waitForRestartedEngine(configDirPath, found.pid, restartWaitMs);
      if (!restarted) {
        console.warn('The engine unit did not come back; starting the bundled engine');
        return null;
      }
      found = restarted;
      reason = await reasonNotToJoin(found, bundled, canReuse, timeoutMs);
      if (reason === UNREACHABLE) return null;
    }
  }
  if (!reason) {
    console.log(`Joining the detached sync engine (pid ${found.pid})`);
    return found.socket;
  }
  console.log(`Stopping the detached sync engine (pid ${found.pid}): ${reason}`);
  try {
    const [, stop] = await engineSocketExchange(
      found.socket,
      [[Method.EngineShutdown, {}]],
      timeoutMs
    );
    if (stop.error) console.warn(`The detached sync engine refused to stop: ${stop.error.message}`);
  } catch (err) {
    // It may exit before answering.
  }
  return null;
}

// How long the launch waits for a restarted engine unit to bind its socket.
export const UNIT_RESTART_WAIT_MS = 30000;
const UNREACHABLE = 'unreachable';

// Null when the launch can join `found`; UNREACHABLE when its socket didn't answer.
async function reasonNotToJoin(
  found: EngineDiscovery,
  bundled: string,
  canReuse: boolean,
  timeoutMs: number
): Promise<string | null> {
  let hello: HelloResult = null;
  try {
    const [answer] = await engineSocketExchange(found.socket, [], timeoutMs);
    hello = answer.result || null;
    if (!hello) {
      console.warn(
        `The detached sync engine refused hello: ${answer.error && answer.error.message}`
      );
    }
  } catch (err) {
    console.warn(`Could not reach the detached sync engine (pid ${found.pid}): ${err.message}`);
    return UNREACHABLE;
  }
  if (!hello) return 'it refused hello';
  if (hello.protocol !== PROTOCOL_VERSION) return `it speaks protocol ${hello.protocol}`;
  if (!hello.build || hello.build !== bundled) return 'it runs another build';
  if (!(hello.features || []).includes(Feature.Accounts)) return 'it keeps no accounts';
  if (!canReuse) return 'the credentials are still in the app';
  return null;
}

// The detached engine that replaced `oldPid` in run/engine.json, once it is bound.
async function waitForRestartedEngine(
  configDirPath: string,
  oldPid: number,
  timeoutMs: number
): Promise<EngineDiscovery | null> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const found = readEngineDiscovery(configDirPath);
    if (found && found.pid !== oldPid && found.detached && pidAlive(found.pid)) return found;
    if (Date.now() >= deadline) return null;
    await new Promise((resolve) => setTimeout(resolve, EARLIER_ENGINE_POLL_MS));
  }
}

/*
Holds will-quit until the supervised engine stopped. The app still holds the single-instance
lock while it waits, so a launch that arrives then (a Dock click, a second instance, a mailto:
link) would open windows in an app about to exit; `deferLaunch` keeps it instead, and the app
relaunches with its URLs and paths once the engine is gone.
*/
export class QuitGate {
  // From the held will-quit until the engine stopped.
  waiting = false;
  done = false;
  // A reset that relaunches the app (application.ts) defers launches too, for its own relaunch.
  relaunching = false;
  _launchArgs: string[] = null;

  // Takes a will-quit: `stop` runs, then `quit` with the launches that arrived meanwhile.
  // False once done, so the quit that follows goes through.
  hold(
    event: { preventDefault(): void },
    stop: () => Promise<unknown>,
    quit: (relaunchArgs: string[] | null) => void
  ) {
    if (this.done) return false;
    event.preventDefault();
    if (this.waiting) return true;
    this.waiting = true;
    stop()
      .catch((err) => console.error(`The sync engine did not stop cleanly: ${err}`))
      .then(() => {
        this.waiting = false;
        this.done = true;
        quit(this._launchArgs);
      });
    return true;
  }

  // True while the app is quitting: the launch happens after the relaunch instead.
  deferLaunch(args: string[] = []) {
    if (!this.waiting && !this.done && !this.relaunching) return false;
    this._launchArgs = (this._launchArgs || []).concat(args);
    return true;
  }

  // The launches deferred so far, for a relaunch the caller does itself; null when none arrived.
  takeLaunchArgs() {
    const args = this._launchArgs;
    this._launchArgs = null;
    return args;
  }
}
