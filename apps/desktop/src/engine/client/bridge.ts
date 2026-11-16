import path from 'path';
import { ipcRenderer, shell } from 'electron';
import { localized } from '../../intl/localize';
import { App } from '../../platform/app-api';

import { Task } from '../../data/tasks/task';
import { IntentTask } from '../../data/tasks/intent';
import TaskQueue from '../../data/stores/task-queue';
import UndoRedoStore from '../../data/stores/undo-redo';

import { Account } from '../../data/models/account';
import { AccountStore } from '../../data/stores/account';
import DatabaseStore from '../../data/database/store';
import OnlineStatusStore from '../../data/stores/online-status';
import { DatabaseChangeRecord } from '../../data/database/change-record';
import DatabaseObjectRegistry from '../../data/database-object-registry';
import { EngineConnection } from './connection';
import EngineEvents, { EngineEvents as EngineEventsType } from './events';
import { wireIntegrationsTransport } from './api';
import { intentErrorMessage, runIntent } from './intents';
import { LogsTailParams, LogsTailResult, Method, THREAD_COUNTS_CLASS } from '../protocol';
import { AccountEvent, EngineStatus, SupervisorChannel } from '../channels';
import * as Actions from '../../data/actions';
import * as Utils from '../../data/models/utils';
import { Model } from '../../data/models/model';

const VERBOSE_UNTIL_KEY = 'core.sync.verboseUntil';

// How much of an account's log the account details show.
const CLIENT_LOG_TAIL_BYTES = 3000;

// Model classes whose delta entries have already failed to inflate this session. The
// same defect repeats on every delta for that class, so it is reported to Sentry once.
const reportedInflationFailures = new Set<string>();

// Inflates one delta's model JSONs, dropping any entry that cannot be inflated (an
// unregistered `__cls`, a malformed row) so a single bad entry never aborts the rest of
// the batch; the delta stream has no retry, so an abort would silently desync the UI.
function convertDeltaModels(modelJSONs: any[], modelClass: string) {
  const models: Model[] = [];
  const rawJSONs: any[] = [];
  for (const json of modelJSONs) {
    try {
      const model = Utils.convertToModel(json);
      if (!model) {
        continue;
      }
      models.push(model);
      rawJSONs.push(json);
    } catch (err) {
      if (!reportedInflationFailures.has(modelClass)) {
        reportedInflationFailures.add(modelClass);
        AppEnv.reportError(err, { modelClass, json });
      }
      console.warn(`Skipping ${modelClass} delta entry that could not be inflated: ${err}`, json);
    }
  }
  return { models, rawJSONs };
}

/*
Applies one batch of delta lines in a window. Every window triggers DatabaseStore and hands
ThreadCounts deltas to its EngineEvents; only the main window runs what must happen once: Task
onSuccess/onError (send sound, error dialogs, send hooks) and ProcessState into
OnlineStatusStore. Rotated secrets never reach a window: serve reports them on its stdin, to the
main process's supervisor.
*/
export function applyDeltas(msgs: string[], { mainWindow }: { mainWindow: boolean }) {
  for (const msg of msgs) {
    if (msg.length === 0) {
      continue;
    }
    if (msg[0] !== '{') {
      if (!msg.startsWith('Waiting for')) {
        console.log(`Sync worker sent non-JSON formatted message: ${msg}`);
      }
      continue;
    }

    let json = null;
    try {
      json = JSON.parse(msg);
    } catch (err) {
      console.log(`Sync worker sent non-JSON formatted message: ${msg}. ${err}`);
      continue;
    }

    const { type, modelJSONs, modelClass } = json;
    if (!modelJSONs || !type || !modelClass) {
      console.log(`Sync worker sent a JSON formatted message with unexpected keys: ${msg}`);
      continue;
    }

    // Note: these deltas don't reflect a real model - we just stream in the process
    // state changes (online / offline) alongside the database changes.
    if (modelClass === 'ProcessState' && modelJSONs.length) {
      if (mainWindow) OnlineStatusStore.onSyncProcessStateReceived(modelJSONs[0]);
      continue;
    }
    // Not a model either: ThreadCountsStore applies them where it runs.
    if (modelClass === THREAD_COUNTS_CLASS) {
      EngineEvents.threadCounts({ type, rows: modelJSONs });
      continue;
    }

    const { models, rawJSONs } = convertDeltaModels(modelJSONs, modelClass);
    if (models.length === 0) {
      continue;
    }
    const record = new DatabaseChangeRecord({
      type, // TODO BG move to "model" naming style, finding all uses might be tricky
      objectClass: modelClass,
      objects: models,
      objectsRawJSON: rawJSONs,
    });
    DatabaseStore.trigger(record);
    if (mainWindow) runTaskHandlers(record);
  }
}

// The window's EngineEvents follows its connection: the features and capabilities of each hello
// and requests on it until the connection drops, and the notifications.
export function feedEngineEvents(
  connection: EngineConnection,
  events: EngineEventsType = EngineEvents
) {
  connection.on('connected', (hello) =>
    events.connected(
      (hello && hello.features) || [],
      (method, params) => connection.request(method, params),
      (hello && hello.capabilities) || null
    )
  );
  connection.on('disconnected', () => events.gone());
  connection.on('subscribed', (answer) => events.subscribed(answer));
  connection.on('task-queued', (task) => events.taskQueued(task));
  connection.on('mail-new', (params) => events.mailNew(params));
  connection.on('notification', ({ method, params }) => events.notification(method, params));
}

// Runs task success / error handlers if the task is now complete.
function runTaskHandlers(record: DatabaseChangeRecord<Model>) {
  // Note: cannot use `record.objectClass` because of subclass names
  if (record.type === 'persist' && record.objects[0] instanceof Task) {
    for (const obj of record.objects) {
      const task = obj as Task;
      if (task.status !== 'complete') {
        continue;
      }
      if (task.error != null) {
        task.onError(task.error);
      } else {
        task.onSuccess();
      }
    }
  }
}

/*
The windows' side of the sync engine. The main process supervises the `serve` engine
(engine/supervisor/supervisor.ts) and every window reads deltas on its own socket connection; the
main window also sends its accounts' commands on it and applies the supervisor's account events.
*/
export default class EngineBridge {
  // Null in spec mode, where no engine runs.
  _connection: EngineConnection | null = null;
  _mainWindow = false;
  _appliedEvents = new Set<number>();
  _lastWait = 0;

  constructor() {
    if (AppEnv.inSpecMode()) return;
    this._mainWindow = AppEnv.isMainWindow();
    this._connection = this._createConnection();
    wireIntegrationsTransport();

    AppEnv.onReadyToUnload(this._onReadyToUnload);
    if (!this._mainWindow) return;

    ipcRenderer.on('run-calendar-sync', (_event, accountId?: string) =>
      this.sendSyncCalendarNow(accountId)
    );

    Actions.queueTask.listen(this._onQueueTask, this);
    Actions.queueTasks.listen(this._onQueueTasks, this);
    Actions.cancelTask.listen(this._onCancelTask, this);
    Actions.fetchBodies.listen(this._onFetchBodies, this);

    AppEnv.onBeforeUnload(this._onBeforeUnload);

    this._attachToSupervisor();
  }

  // Public

  openLogs() {
    const { configDirPath } = AppEnv.getLoadSettings();
    const configDirItem = path.join(configDirPath, 'config.json');
    shell.showItemInFolder(configDirItem);
  }

  // The supervisor follows the setting and passes it to the engine (engine.setLogLevel).
  toggleVerboseLogging() {
    const { configDirPath } = AppEnv.getLoadSettings();
    let message = localized('Thank you for helping debug Airpost.');
    let phrase = 'disabled';

    if (AppEnv.config.get(VERBOSE_UNTIL_KEY)) {
      AppEnv.config.set(VERBOSE_UNTIL_KEY, 0);
    } else {
      AppEnv.config.set(VERBOSE_UNTIL_KEY, Date.now() + 30 * 60 * 1000);
      phrase = 'enabled';
      message =
        `Verbose logging will be enabled for the next thirty minutes. This records ` +
        `all network traffic to your mail providers and will be quite slow. Wait for your ` +
        `problem to occur, and then submit the engine-*.log files located in the ` +
        `directory: \n\n${configDirPath}.`;
    }
    AppEnv.showErrorDialog({
      title: localized(`Verbose logging is now %@`, phrase),
      message,
    });
  }

  async forceRelaunchClient(account: Account) {
    await ipcRenderer.invoke(SupervisorChannel.ForceRelaunch, account.id);
  }

  // The end of the account's log, from its first whole line; empty when the engine can't say.
  async tailClientLog(accountId: string) {
    try {
      const { text, truncated } = await this._conn().request<LogsTailResult>(
        Method.EngineLogsTail,
        { accountId, bytes: CLIENT_LOG_TAIL_BYTES } as LogsTailParams
      );
      return truncated ? text.substr(text.indexOf('\n') + 1) : text;
    } catch (err) {
      console.warn(`Could not read the sync log of account ${accountId}: ${err}`);
      return '';
    }
  }

  sendSyncMailNow() {
    console.warn('Sending `wake` to all engine workers...');
    this._conn()
      .request(Method.SyncWake, {})
      .catch((err) => console.warn(`sync.wake failed: ${err.message}`));
  }

  // Only the main window sends account commands; other windows route through the main
  // process, which forwards `run-calendar-sync` to the main window.
  sendSyncCalendarNow(accountId?: string) {
    if (!AppEnv.isMainWindow()) {
      ipcRenderer.send('command', 'application:sync-calendar', accountId);
      return;
    }
    const connection = this._conn();
    const ids = accountId ? [accountId] : connection.accountIds();
    for (const id of ids) {
      if (connection.accountIds().includes(id)) {
        connection.send(id, { type: 'sync-calendar' });
      }
    }
  }

  sendMessageToAccount(accountId: string, json: Record<string, unknown>) {
    if (!this._conn().send(accountId, json)) {
      const { emailAddress } = AccountStore.accountForId(accountId) || { emailAddress: undefined };
      return AppEnv.showErrorDialog({
        title: localized(`Airpost is unable to sync %@`, emailAddress),
        message: localized(
          `In order to perform actions on this mailbox, you need to resolve the sync issue. Visit Preferences > Accounts for more information.`
        ),
      });
    }
  }

  // `keepMetadata: false` when the account is being removed: its plugin metadata goes too.
  async resetCacheForAccount(
    account: Account,
    { silent, keepMetadata }: { silent?: boolean; keepMetadata?: boolean } = {}
  ) {
    // The supervisor drops the account from the engine, resets it and adds it back.
    const reset = async () => {
      const options = keepMetadata === false ? { keepMetadata } : {};
      const result = await ipcRenderer.invoke(SupervisorChannel.ResetCache, account.id, options);
      if (result && result.error) throw new Error(result.error);
    };

    if (!silent) {
      AppEnv.showErrorDialog({
        title: localized(`Cleanup Started`),
        message: localized(
          `Airpost is clearing its cache %@. Depending on the size of the mailbox, this may take a few seconds or a few minutes. An alert will appear when cleanup is complete.`,
          account.emailAddress
        ),
      });
    }

    try {
      const start = Date.now();

      await reset();

      if (!silent) {
        AppEnv.showErrorDialog({
          title: localized(`Cleanup Complete`),
          message: localized(
            `Airpost reset the local cache for %@ in %@ seconds. Your mailbox will now begin to sync again.`,
            account.emailAddress,
            Math.ceil((Date.now() - start) / 1000)
          ),
        });
      }
    } catch (error) {
      AppEnv.showErrorDialog({
        title: localized(`Cleanup Error`),
        message: localized(`Airpost was unable to reset the local cache. %@`, error),
      });
    }
  }

  // Private

  _conn() {
    if (!this._connection) throw new Error('No sync engine connection in spec mode');
    return this._connection;
  }

  _createConnection() {
    const connection = new EngineConnection({
      clientName: `airpost-desktop/${AppEnv.getVersion()}`,
      mainWindow: this._mainWindow,
    });
    connection.on('deltas', this._onIncomingMessages);
    feedEngineEvents(connection);
    let statusSeen = false;
    ipcRenderer.on(SupervisorChannel.Status, (_event, status: EngineStatus) => {
      statusSeen = true;
      connection.setStatus(status);
      EngineEvents.supervisorStatus(status);
    });
    // A Status that arrives first is newer than this answer.
    ipcRenderer.invoke(SupervisorChannel.GetStatus).then(
      (status: EngineStatus | null) => {
        if (statusSeen) return;
        connection.setStatus(status);
        EngineEvents.supervisorStatus(status);
      },
      (err) => console.error(`Could not ask the main process for the sync engine: ${err}`)
    );
    return connection;
  }

  // The supervisor's account events: the ones this window's renderer missed, then each as it
  // happens. A renderer started by a reload may see one both ways, so each applies once.
  _attachToSupervisor() {
    ipcRenderer.on(
      SupervisorChannel.AccountEvent,
      (_event, accountEvent: AccountEvent & { seq: number }) =>
        this._applyAccountEvents([accountEvent])
    );
    ipcRenderer.invoke(SupervisorChannel.AttachMainWindow).then(
      (missed: Array<AccountEvent & { seq: number }>) => this._applyAccountEvents(missed),
      (err) => console.error(`Could not attach to the sync engine supervisor: ${err}`)
    );
  }

  _applyAccountEvents(events: Array<AccountEvent & { seq: number }>) {
    let last = 0;
    for (const accountEvent of events) {
      last = Math.max(last, accountEvent.seq);
      if (this._appliedEvents.has(accountEvent.seq)) continue;
      this._appliedEvents.add(accountEvent.seq);
      this._onAccountEvent(accountEvent);
    }
    if (last) ipcRenderer.send(SupervisorChannel.AckAccountEvents, last);
  }

  _onAccountEvent(accountEvent: AccountEvent) {
    switch (accountEvent.type) {
      case 'launched':
        if (accountEvent.syncState !== Account.SYNC_STATE_OK) {
          Actions.updateAccount(accountEvent.accountId, {
            syncState: Account.SYNC_STATE_OK,
            syncError: null,
          });
        }
        return;
      case 'exited':
        OnlineStatusStore.onSyncProcessExited(accountEvent.accountId);
        return;
      case 'too-many-failures': {
        const { code, signal, error } = accountEvent.exit;
        let syncError: Error | null = null;
        if (error) {
          syncError = new Error(error.message);
          if (error.syncError) (syncError as any).syncError = error.syncError;
        }
        Actions.updateAccount(accountEvent.accountId, {
          syncState: accountEvent.authFailure
            ? Account.SYNC_STATE_AUTH_FAILED
            : Account.SYNC_STATE_ERROR,
          syncError: { code, error: syncError, signal },
        });
        return;
      }
    }
  }

  _onQueueTask(task: Task) {
    if (!DatabaseObjectRegistry.isInRegistry(task.constructor.name)) {
      console.log(task);
      throw new Error(
        'You must queue a `Task` instance which is registred with the DatabaseObjectRegistry'
      );
    }
    if (!task.id) {
      console.log(task);
      throw new Error(
        'Tasks must have an ID prior to being queued. Check that your Task constructor is calling `super`'
      );
    }
    if (!task.accountId) {
      throw new Error(
        `Tasks must have an accountId. Check your instance of ${task.constructor.name}.`
      );
    }

    task.willBeQueued();

    if (task instanceof IntentTask) {
      this._runIntent(task);
      return;
    }

    task.status = 'local';
    (task as any).origin = (new Error().stack ?? '').split('\n').slice(2).join('\n');

    this.sendMessageToAccount(task.accountId, { type: 'queue-task', task: task });
  }

  // The engine queues the intent's tasks; they and the undo entry arrive like any other.
  _runIntent(task: IntentTask) {
    runIntent(task.intent, task.params).then(
      (result) => UndoRedoStore.describe(result.undo, task.description()),
      (err) =>
        AppEnv.showErrorDialog(
          localized('Sorry, this change could not be made. %@', intentErrorMessage(err))
        )
    );
  }

  _onQueueTasks(tasks: Task[]) {
    if (!tasks || !tasks.length) {
      return;
    }
    for (const task of tasks) {
      this._onQueueTask(task);
    }
  }

  _onCancelTask(taskOrId: Task | string) {
    let task: Task | undefined;
    if (typeof taskOrId === 'string') {
      task = TaskQueue.queue().find((t) => t.id === taskOrId);
    } else {
      task = taskOrId;
    }
    if (task) {
      this.sendMessageToAccount(task.accountId, { type: 'cancel-task', taskId: task.id });
    }
  }

  _onIncomingMessages = (msgs: string[]) => {
    applyDeltas(msgs, { mainWindow: this._mainWindow });
  };

  _onFetchBodies(messages: { id: string; accountId: string }[]) {
    const byAccountId: { [accountId: string]: string[] } = {};
    for (const msg of messages) {
      byAccountId[msg.accountId] = byAccountId[msg.accountId] || [];
      byAccountId[msg.accountId].push(msg.id);
    }
    for (const accountId of Object.keys(byAccountId)) {
      this.sendMessageToAccount(accountId, { type: 'need-bodies', ids: byAccountId[accountId] });
    }
  }

  _onBeforeUnload = (readyToUnload: () => void) => {
    // If other windows are open, delay the closing of the main window
    // by 400ms the first time beforeUnload is called so other windows
    // ave a chance to save drafts before we kill the workers.
    if (App.openWindowCount() <= 1) {
      return true;
    }
    if (this._lastWait && Date.now() - this._lastWait < 2000) {
      return true;
    }
    this._lastWait = Date.now();
    setTimeout(readyToUnload, 400);
    return false;
  };

  // The `serve` engine outlives the window; the supervisor stops it when the app quits.
  _onReadyToUnload = () => {
    this._conn().close();
  };
}
