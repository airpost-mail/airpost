// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The supervisor of the Rust engine's `serve` mode: one engine process for every account
(docs/engine-protocol-plan.md section 4). The main process's EngineSupervisor owns it when
serveEngineSelected() is true; the main window's MailsyncBridge keeps its per-account
processes otherwise.

It keeps the engine's accounts equal to the host's (account.add / account.remove), routes
each account's commands to it, and applies the per-process relaunch rules of the per-account
path to accounts instead: an `account.state` `stopped` counts as that account's crash and adds
it again, until CrashTracker's cutoff. An engine exit counts as an engine crash and respawns
it with every account. Nothing here touches AppEnv or the stores, so the spec drives it with a
fake engine.
*/
import type { EngineServeProcess, MailsyncProcessExit } from './mailsync-process';
import {
  AccountAddResult,
  AccountState,
  AccountStateParams,
  ErrorCode,
  EXIT_SOCKET_IN_USE,
  serveSocketPath,
  SyncErrorJSON,
} from './engine-protocol';

const MAX_CRASH_HISTORY = 10;
const CRASH_WINDOW_MS = 5 * 60 * 1000;
const CRASH_LIMIT = 5;

// After a reset's account.remove reported threads still in a blocking call, how long to wait
// for them before the one-shot reset deletes the account's rows. The engine cannot report
// when they end, since the account is gone from it.
export const RESET_STRAGGLER_WAIT_MS = 10000;

// Engine respawns after the first in a row back off up to this.
export const ENGINE_RESPAWN_MAX_DELAY_MS = 8000;

// A window reload or relaunch starts the new engine while the old one may still be shutting
// down on the socket. An engine that exits because the socket is in use is spawned again this
// often, this many times in a row, before such exits count as crashes.
export const ENGINE_IN_USE_RETRY_MS = 1000;
export const ENGINE_IN_USE_RETRIES = 30;

export interface CrashKeyJSON {
  id: string;
  settings: Record<string, unknown>;
}

/*
This class keeps track of how often Mailsync workers crash. If a mailsync
worker exits more than 5 times in <5 minutes, we consider it "too many failures"
and won't relaunch it until:

- the user restarts the app, clearing the history
- the user changes the account's settings (updating password, etc.)
- the user explicitly says "Try Again" in the UI

*/
export class CrashTracker {
  _timestamps = {};
  _tooManyFailures = {};

  forgetCrashes(fullAccountJSON: CrashKeyJSON) {
    const key = this._keyFor(fullAccountJSON);
    delete this._timestamps[key];
    delete this._tooManyFailures[key];
  }

  recordClientCrash(fullAccountJSON: CrashKeyJSON, crash: MailsyncProcessExit) {
    console.log(`Sync worker exited.`, crash);

    this._appendCrashToHistory(fullAccountJSON);

    // We now let crashpad do this, because Sentry was losing it's mind.
  }

  _keyFor({ id, settings }) {
    return JSON.stringify({ id, settings });
  }

  _appendCrashToHistory(fullAccountJSON: CrashKeyJSON) {
    const key = this._keyFor(fullAccountJSON);
    this._timestamps[key] = this._timestamps[key] || [];
    if (this._timestamps[key].unshift(Date.now()) > MAX_CRASH_HISTORY) {
      this._timestamps[key].length = MAX_CRASH_HISTORY;
    }

    // has the client crashed more than 5 times in the last 5 minutes?
    // If so, do not restart. We'll mark that the account is not syncing.
    if (
      this._timestamps[key].length >= CRASH_LIMIT &&
      Date.now() - this._timestamps[key][CRASH_LIMIT - 1] < CRASH_WINDOW_MS
    ) {
      this._tooManyFailures[key] = true;
    }
  }

  tooManyFailures(fullAccountJSON: CrashKeyJSON) {
    const key = this._keyFor(fullAccountJSON);
    return this._tooManyFailures[key];
  }

  recentCrashes(fullAccountJSON: CrashKeyJSON) {
    const now = Date.now();
    const times: number[] = this._timestamps[this._keyFor(fullAccountJSON)] || [];
    return times.filter((t) => now - t < CRASH_WINDOW_MS).length;
  }
}

// Whether a sync failure means the credentials were refused.
export function isAuthFailure(error: unknown) {
  const text = `${error}`;
  return (
    text.includes('Response Code: 401') || // mailspring services
    text.includes('Response Code: 403') || // mailspring services
    text.includes('invalid_grant') || // Google
    text.includes('ErrorAuthentication') // mailcore
  );
}

/*
Whether the main window runs one `serve` engine for every account: the Rust engine on macOS
and Linux, unless TERN_ENGINE_PROCESS=per-account (phase A's per-account processes) or
TERN_ENGINE_TRANSPORT=stdio (per-account on stdio), or no socket path fits.
*/
export function serveEngineSelected(syncEngine: string, configDirPath: string) {
  if (syncEngine !== 'rust' || process.platform === 'win32') return false;
  if (process.env.TERN_ENGINE_PROCESS === 'per-account') return false;
  if (process.env.TERN_ENGINE_TRANSPORT === 'stdio') return false;
  return serveSocketPath(configDirPath) !== null;
}

export interface EngineAccount {
  id: string;
}

// The account JSON with its secrets, as account.add takes it.
export interface AccountJSON extends CrashKeyJSON {
  [key: string]: any;
}

export interface EngineClientHost {
  // A configured engine, not yet started.
  createEngine(): EngineServeProcess;
  // The accounts that should be syncing (config.json `accounts`).
  accounts(): EngineAccount[];
  accountJSONs(accounts: EngineAccount[]): Promise<AccountJSON[]>;
  onDeltas(msgs: string[]): void;
  // The engine no longer runs the account: it stopped, was removed, or the engine exited.
  onAccountExited(accountId: string): void;
  // account.add was sent for this account.
  onAccountLaunched(json: AccountJSON): void;
  // The account (or, after an engine cutoff, every account) is not relaunched any more.
  onTooManyFailures(accountId: string, exit: MailsyncProcessExit, authFailure: boolean): void;
  // `entries` or `held` changed: an account is being added, was added, is being removed or
  // is gone, or a reset holds it.
  onAccountsChanged?(): void;
}

interface Entry {
  state: 'adding' | 'added' | 'removing';
  json: AccountJSON | null;
  // Commands sent before account.add answered, in order; null once it has.
  pending: Array<{ type?: string; [key: string]: any }> | null;
  removal: Promise<string[]> | null;
}

// The last account.remove of an account the engine no longer runs, for a reset that comes
// after it finished: its threads still running, and when it answered.
interface Removal {
  done: Promise<string[]>;
  finishedAt: number | null;
}

const ENGINE_CRASH_KEY: CrashKeyJSON = { id: 'engine', settings: {} };

export class EngineClient {
  host: EngineClientHost;
  engine: EngineServeProcess = null;
  // The accounts the engine was asked to run, by id.
  entries = new Map<string, Entry>();
  // Accounts being reset or relaunched, which ensure() leaves alone.
  held = new Map<string, number>();
  removals = new Map<string, Removal>();
  // Engines in a row that exited because the socket was in use.
  inUseExits = 0;
  crashes = new CrashTracker();
  engineCrashes = new CrashTracker();
  engineGaveUp = false;
  shuttingDown = false;
  respawnTimer = null;

  constructor(host: EngineClientHost) {
    this.host = host;
  }

  // Adds the accounts the engine is missing and removes the ones the host no longer lists.
  ensure(): Promise<void> {
    if (this.shuttingDown || this.engineGaveUp || this.respawnTimer) {
      return Promise.resolve();
    }
    const wanted = this.host.accounts();
    const wantedIds = new Set(wanted.map((a) => a.id));
    for (const id of [...this.entries.keys()]) {
      if (!wantedIds.has(id) && !this.held.has(id)) {
        this._remove(id);
      }
    }
    const missing = wanted.filter((a) => !this.entries.has(a.id) && !this.held.has(a.id));
    return missing.length ? this._add(missing, { force: false }) : Promise.resolve();
  }

  // "Try Again": forgets the account's crashes (and the engine's, if it gave up) and adds it
  // again, removing it first if the engine still runs it.
  async forceRelaunch(account: EngineAccount) {
    if (this.shuttingDown) return;
    if (this.engineGaveUp) {
      this.engineGaveUp = false;
      this.inUseExits = 0;
      this.engineCrashes.forgetCrashes(ENGINE_CRASH_KEY);
    }
    this._hold(account.id);
    try {
      await this._remove(account.id);
      await this._add([account], { force: true });
    } finally {
      this._release(account.id);
    }
    await this.ensure();
  }

  // Takes the account out of the engine, runs `reset` (the one-shot `--mode reset`), then lets
  // ensure() add it back if the host still lists it.
  async resetCache<T>(accountId: string, reset: () => Promise<T>): Promise<T> {
    this._hold(accountId);
    try {
      // ensure() may have removed it already, as when the account is deleted.
      const earlier = !this.entries.has(accountId) && this.removals.get(accountId);
      const stillRunning = await (earlier ? earlier.done : this._remove(accountId));
      if (stillRunning.length) {
        const finishedAt = earlier && earlier.finishedAt;
        const elapsed = finishedAt ? Date.now() - finishedAt : 0;
        const wait = Math.max(0, RESET_STRAGGLER_WAIT_MS - elapsed);
        console.warn(
          `Waiting for ${stillRunning.join(', ')} of account ${accountId} before resetting it`
        );
        if (wait) await new Promise((resolve) => setTimeout(resolve, wait));
      }
      return await reset();
    } finally {
      this._release(accountId);
      this.ensure();
    }
  }

  // Sends one of the bridge's commands to the account. False when the engine doesn't run it,
  // so the bridge can say the account can't sync.
  send(accountId: string, json: { type?: string; [key: string]: any }) {
    const entry = this.entries.get(accountId);
    if (!entry || entry.state === 'removing') {
      if (this.held.has(accountId)) {
        console.warn(`Dropped ${json.type} for account ${accountId} while it is reset`);
        return true;
      }
      return false;
    }
    if (entry.pending) {
      entry.pending.push(json);
    } else {
      this.engine.sendToAccount(accountId, json);
    }
    return true;
  }

  wakeAll() {
    if (this.engine) this.engine.wakeAll();
  }

  syncCalendar(accountId?: string) {
    const ids = accountId ? [accountId] : [...this.entries.keys()];
    for (const id of ids) {
      if (this.entries.has(id)) this.send(id, { type: 'sync-calendar' });
    }
  }

  shutdown() {
    this.shuttingDown = true;
    clearTimeout(this.respawnTimer);
    this.respawnTimer = null;
    const engine = this.engine;
    this.engine = null;
    this.entries.clear();
    this._changed();
    return engine ? engine.shutdown() : Promise.resolve();
  }

  // Private

  _changed() {
    if (this.host.onAccountsChanged) this.host.onAccountsChanged();
  }

  _hold(accountId: string) {
    this.held.set(accountId, (this.held.get(accountId) || 0) + 1);
    this._changed();
  }

  _release(accountId: string) {
    const n = (this.held.get(accountId) || 1) - 1;
    if (n > 0) {
      this.held.set(accountId, n);
    } else {
      this.held.delete(accountId);
    }
    this._changed();
  }

  _ensureEngine() {
    if (this.engine) return this.engine;
    const engine = this.host.createEngine();
    this.engine = engine;
    engine.on('deltas', (msgs: string[]) => this.host.onDeltas(msgs));
    engine.on('account-state', (params: AccountStateParams) => {
      if (this.engine === engine) this._onAccountState(params);
    });
    engine.on('ready', () => {
      if (this.engine === engine) this.inUseExits = 0;
    });
    engine.on('close', (exit: MailsyncProcessExit) => this._onEngineClose(engine, exit));
    try {
      engine.serve();
    } catch (error) {
      this._onEngineClose(engine, { code: -1, error, signal: null });
      return null;
    }
    return engine;
  }

  async _add(accounts: EngineAccount[], { force }: { force: boolean }) {
    if (this.shuttingDown) return;
    const engine = this._ensureEngine();
    if (!engine) return;
    // Claimed before the await, so two calls never add one account twice.
    const claimed = new Map<string, Entry>();
    for (const { id } of accounts) {
      const entry: Entry = { state: 'adding', json: null, pending: [], removal: null };
      this.entries.set(id, entry);
      this.removals.delete(id);
      claimed.set(id, entry);
    }
    this._changed();
    const current = (id: string, entry: Entry) =>
      this.entries.get(id) === entry && entry.state === 'adding';

    let jsons: AccountJSON[] = [];
    try {
      jsons = await this.host.accountJSONs(accounts);
    } finally {
      for (const [id, entry] of claimed) {
        if (current(id, entry) && !jsons.some((j) => j.id === id)) this.entries.delete(id);
      }
      this._changed();
    }
    for (const json of jsons) {
      const entry = claimed.get(json.id);
      if (!entry || !current(json.id, entry)) continue;
      if (force) {
        this.crashes.forgetCrashes(json);
      } else if (this.crashes.tooManyFailures(json)) {
        this.entries.delete(json.id);
        continue;
      }
      entry.json = json;
      engine.addAccount(json, null).then(
        (result) => this._added(json.id, entry, result),
        (err) => this._addFailed(json.id, entry, err)
      );
      this.host.onAccountLaunched(json);
    }
    this._changed();
  }

  // account.add answered: the commands held for it go out. An account that already stopped
  // while starting is a crash here, since its account.state may not reach this client (the
  // hub keeps its backlog for one subscriber); a later one finds the entry gone.
  _added(accountId: string, entry: Entry, result?: AccountAddResult) {
    if (this.entries.get(accountId) !== entry || entry.state !== 'adding') return;
    if (result && result.state === AccountState.Stopped) {
      this._stopped(accountId, entry, this._stoppedError(result.error));
      return;
    }
    entry.state = 'added';
    const pending = entry.pending;
    entry.pending = null;
    for (const json of pending) {
      this.engine.sendToAccount(accountId, json);
    }
    this._changed();
  }

  _addFailed(accountId: string, entry: Entry, err: any) {
    if (this.entries.get(accountId) !== entry || entry.state !== 'adding') return;
    // The engine's close relaunches everything.
    if (err && err.engineExited) return;
    if (err && err.code === ErrorCode.AccountExists) {
      this._added(accountId, entry);
      return;
    }
    // Refused (invalid account JSON, say): a crash, as a sync process exiting on it was.
    console.warn(`The sync engine refused account ${accountId}: ${err && err.message}`);
    this._stopped(accountId, entry, new Error(`${err && err.message}`));
  }

  _onAccountState({ accountId, state, error }: AccountStateParams) {
    const entry = this.entries.get(accountId);
    if (!entry || entry.state === 'removing' || state !== AccountState.Stopped) return;
    this._stopped(accountId, entry, this._stoppedError(error));
  }

  // The `error` of the MailsyncProcessExit a stopped account is recorded with: its text is the
  // SyncError's key and debuginfo, as the per-account path's stderr carried them, and it keeps
  // the SyncError JSON for the account's syncError.
  _stoppedError(error?: SyncErrorJSON) {
    const strip = (text: string) => (this.engine ? this.engine._stripSecrets(text) : text);
    const text = error ? `${error.key}: ${error.debuginfo || ''}` : 'The account stopped';
    const result = new Error(strip(text));
    if (error) {
      (result as any).syncError = JSON.parse(strip(JSON.stringify(error)));
    }
    return result;
  }

  _stopped(accountId: string, entry: Entry, error: Error) {
    this.entries.delete(accountId);
    this._changed();
    this.host.onAccountExited(accountId);
    if (!entry.json) return;
    const exit: MailsyncProcessExit = { code: null, error, signal: null };
    this.crashes.recordClientCrash(entry.json, exit);
    if (this.crashes.tooManyFailures(entry.json)) {
      this.host.onTooManyFailures(accountId, exit, isAuthFailure(error));
    } else {
      this.ensure();
    }
  }

  _remove(accountId: string): Promise<string[]> {
    const entry = this.entries.get(accountId);
    if (!entry) return Promise.resolve([]);
    if (entry.removal) return entry.removal;
    entry.state = 'removing';
    entry.pending = null;
    this._changed();
    const engine = this.engine;
    const removed = engine
      ? engine.removeAccount(accountId).then(
          (result) => (result && result.stillRunning) || [],
          (err) => {
            if (!err.engineExited && err.code !== ErrorCode.UnknownAccount) {
              console.warn(`account.remove ${accountId} failed: ${err.message}`);
            }
            return [] as string[];
          }
        )
      : Promise.resolve([] as string[]);
    const removal: Removal = { done: null, finishedAt: null };
    entry.removal = removed.then((stillRunning) => {
      removal.finishedAt = Date.now();
      if (this.entries.get(accountId) === entry) this.entries.delete(accountId);
      this._changed();
      this.host.onAccountExited(accountId);
      return stillRunning;
    });
    removal.done = entry.removal;
    this.removals.set(accountId, removal);
    return entry.removal;
  }

  _onEngineClose(engine: EngineServeProcess, exit: MailsyncProcessExit) {
    if (this.engine !== engine) return;
    this.engine = null;
    const ids = [...this.entries.keys()];
    this.entries.clear();
    this._changed();
    for (const id of ids) {
      this.host.onAccountExited(id);
    }
    // Killed from outside, as a per-account process could be: the next ensure() respawns it.
    if (this.shuttingDown || exit.signal === 'SIGTERM') return;

    if (exit.code === EXIT_SOCKET_IN_USE && this.inUseExits < ENGINE_IN_USE_RETRIES) {
      this.inUseExits += 1;
      this.respawnTimer = setTimeout(() => {
        this.respawnTimer = null;
        this.ensure();
      }, ENGINE_IN_USE_RETRY_MS);
      return;
    }

    this.engineCrashes.recordClientCrash(ENGINE_CRASH_KEY, exit);
    if (this.engineCrashes.tooManyFailures(ENGINE_CRASH_KEY)) {
      this.engineGaveUp = true;
      const authFailure = isAuthFailure(exit.error);
      for (const { id } of this.host.accounts()) {
        this.host.onTooManyFailures(id, exit, authFailure);
      }
      return;
    }
    const recent = this.engineCrashes.recentCrashes(ENGINE_CRASH_KEY);
    const delay = recent <= 1 ? 0 : Math.min(1000 * 2 ** (recent - 2), ENGINE_RESPAWN_MAX_DELAY_MS);
    this.respawnTimer = setTimeout(() => {
      this.respawnTimer = null;
      this.ensure();
    }, delay);
  }
}
