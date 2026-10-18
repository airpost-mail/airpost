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
*/
import fs from 'fs';
import path from 'path';
import { AccountJSON, EngineAccount, EngineClient } from '../engine-client';
import { EngineServeProcess, MailsyncProcessExit, ResetOptions } from '../mailsync-process';
import { AccountSecretsJSON, KeyManager } from './key-manager';
import { SettingsValues } from '../engine-protocol';
import type { EngineSettingsMirror } from './engine-settings';
import {
  AccountEvent,
  AccountTestReply,
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

  constructor(options: EngineSupervisorOptions) {
    this.options = options;
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
    });
  }

  start() {
    const { config } = this.options;
    this._configSubscriptions = [
      config.onDidChange('accounts', () => this.ensure()),
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

  // `account` carries its secrets: onboarding tests settings it has not saved yet.
  testAccount(account: AccountSecretsJSON) {
    return this.client.withEngine((engine) => engine.testAccount(account));
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
    const generation = ++this._generation;
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

  async _accountJSONs(accounts: EngineAccount[]): Promise<AccountJSON[]> {
    const { keyManager } = this.options;
    const byId = new Map(this.accounts().map((json) => [json.id, json]));
    const keys = await keyManager._getKeyHash();
    const result: AccountJSON[] = [];
    for (const { id } of accounts) {
      const json = byId.get(id);
      if (json)
        result.push((await keyManager.insertAccountSecrets(json, keys)) as unknown as AccountJSON);
    }
    return result;
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
  ipcMain.handle(
    SupervisorChannel.TestAccount,
    async (_event, account: AccountSecretsJSON): Promise<AccountTestReply> => {
      try {
        return { result: await supervisor.testAccount(account) };
      } catch (err) {
        return { error: (err && err.message) || `${err}` };
      }
    }
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
