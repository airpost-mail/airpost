import UndoRedoStore from '../../src/data/stores/undo-redo';
import TaskQueue from '../../src/data/stores/task-queue';
import EngineEvents from '../../src/engine/client/events';
import * as Actions from '../../src/data/actions';
import { ChangeStarredTask } from '../../src/data/tasks/change-starred';
import { IntentTask } from '../../src/data/tasks/intent';
import { SyncbackMetadataTask } from '../../src/data/tasks/syncback-metadata';
import { Message } from '../../src/data/models/message';
import { Thread } from '../../src/data/models/thread';
import * as Utils from '../../src/data/models/utils';
import { SyncbackEventTask } from '../../src/data/tasks/syncback-event';

// Lets a chain of already-resolved promises settle without relying on real timers.
const flushPromises = () => new Promise<void>((resolve) => window.originalSetTimeout(resolve, 0));

// A v1 plugin's own task class, which `undo.record` doesn't take.
class PluginStarTask extends ChangeStarredTask {}

function entry(token: string, more: Record<string, unknown> = {}): any {
  return {
    token,
    client: 'airpost-desktop',
    action: 'archive',
    label: 'Archived 2 threads',
    count: 2,
    at: 1,
    accountIds: ['ac-1'],
    taskIds: ['task-1'],
    state: 'done',
    canUndo: true,
    ...more,
  };
}

describe('UndoRedoStore', function () {
  let requests: Array<{ method: string; params: any }>;
  let answers: { [method: string]: any };
  let queued: any[][];

  const changed = (e: any) => EngineEvents.notification('undo.changed', { entry: e });

  beforeEach(function () {
    answers = { 'undo.list': { entries: [] } };
    requests = [];
    // Connected without a hello, which would read the log.
    EngineEvents.features = new Set(['undo', 'intents']);
    EngineEvents._request = (method, params) => {
      requests.push({ method, params });
      const answer = answers[method];
      if (answer instanceof Error) return Promise.reject(answer);
      return Promise.resolve(answer === undefined ? {} : answer);
    };
    queued = [];
    spyOn(Actions, 'queueTasks').andCallFake((tasks) => queued.push(tasks));
    spyOn(AppEnv, 'reportError');
    spyOn(AppEnv, 'showErrorDialog');
    spyOn(console, 'warn');

    UndoRedoStore._undo = [];
    UndoRedoStore._redo = [];
    UndoRedoStore._mostRecentBlock = null;
    UndoRedoStore._descriptions.clear();
  });

  afterEach(() => {
    EngineEvents.features = null;
    EngineEvents._request = null;
  });

  it("keeps the app's own entries, worded from their action and count", function () {
    changed(entry('u1'));
    changed(entry('u2', { client: 'airpost-cli' }));

    expect(UndoRedoStore._undo.length).toBe(1);
    expect(UndoRedoStore.getMostRecent().description).toBe('Archived 2 threads');
    expect(UndoRedoStore.getMostRecent().entry.token).toBe('u1');
  });

  it("undoes and redoes with the entry's token", async function () {
    changed(entry('u1'));
    UndoRedoStore.undo();
    await flushPromises();
    expect(requests.map((r) => [r.method, r.params.token])).toEqual([['undo.apply', 'u1']]);
    expect(UndoRedoStore.getMostRecent()).toBe(null);
    expect(UndoRedoStore._redo.length).toBe(1);

    changed(entry('u1', { state: 'undone', canUndo: false, canRedo: true }));
    expect(UndoRedoStore._redo.length).toBe(1);

    UndoRedoStore.redo();
    await flushPromises();
    expect(requests[1].method).toBe('undo.redo');
    expect(requests[1].params.token).toBe('u1');
    expect(UndoRedoStore._undo.length).toBe(1);
  });

  it('follows an entry undone by another client and one that leaves the log', function () {
    changed(entry('u1'));
    changed(entry('u1', { state: 'undone', canRedo: true }));
    expect(UndoRedoStore._undo.length).toBe(0);
    expect(UndoRedoStore._redo.length).toBe(1);
    expect(UndoRedoStore.getMostRecent()).toBe(null);

    changed(entry('u1', { state: 'gone', reason: 'failed' }));
    expect(UndoRedoStore._redo.length).toBe(0);
  });

  it('reads the log again when the engine refuses an undo', async function () {
    changed(entry('u1'));
    answers['undo.apply'] = new Error("entry u1 can't be undone now");
    UndoRedoStore.undo();
    await flushPromises();
    await flushPromises();

    expect(requests.map((r) => r.method)).toEqual(['undo.apply', 'undo.list']);
    expect(UndoRedoStore._redo.length).toBe(0);
    expect(AppEnv.showErrorDialog).not.toHaveBeenCalled();
  });

  it('takes a description given before or after the entry', function () {
    UndoRedoStore.describe('u1', 'Moved to Receipts');
    changed(entry('u1', { action: 'move', label: 'Moved 2 threads to f-1' }));
    expect(UndoRedoStore.getMostRecent().description).toBe('Moved to Receipts');

    changed(entry('u2', { action: 'label', label: 'Changed labels of 1 thread' }));
    UndoRedoStore.describe('u2', 'Added Work');
    expect(UndoRedoStore.getMostRecent().description).toBe('Added Work');
  });

  it('seeds the stacks from undo.list after a hello', async function () {
    answers['undo.list'] = {
      entries: [
        entry('u3', { state: 'undone', canUndo: false, canRedo: true }),
        entry('u2', { client: 'airpost-mcp' }),
        entry('u1'),
      ],
    };
    UndoRedoStore._load();
    await flushPromises();

    expect(requests[0]).toEqual({ method: 'undo.list', params: { mine: true } });
    expect(UndoRedoStore._undo.map((b) => b.entry.token)).toEqual(['u1']);
    expect(UndoRedoStore._redo.map((b) => b.entry.token)).toEqual(['u3']);
    expect(UndoRedoStore.getMostRecent()).toBe(null);
  });

  it('records tasks queued with canBeUndone once their local phases ran', async function () {
    const task = new ChangeStarredTask({ threadIds: ['t1'], accountId: 'ac-1', starred: true });
    const local = new ChangeStarredTask({ ...task.toJSON(), status: 'remote' } as any);
    spyOn(TaskQueue, 'waitForPerformLocal').andReturn(Promise.resolve(local));
    answers['undo.record'] = { token: 'u9' };

    UndoRedoStore._onQueue(task);
    await flushPromises();

    expect(requests).toEqual([
      {
        method: 'undo.record',
        params: { tasks: [{ accountId: 'ac-1', taskId: task.id }], label: task.description() },
      },
    ]);
    // The block is the entry, when it arrives.
    expect(UndoRedoStore._undo.length).toBe(0);
  });

  it('leaves intents and tasks without canBeUndone to the engine', async function () {
    const thread = new Thread({ id: 't1', accountId: 'ac-1' });
    spyOn(TaskQueue, 'waitForPerformLocal');
    UndoRedoStore._onQueue([IntentTask.forThreads('threads.archive', [thread])]);
    UndoRedoStore._onQueue(
      new ChangeStarredTask({ threads: [thread], starred: true, canBeUndone: false })
    );
    await flushPromises();

    expect(TaskQueue.waitForPerformLocal).not.toHaveBeenCalled();
    expect(requests).toEqual([]);
  });

  it("reverses a plugin's own task class in the app", async function () {
    const task = new PluginStarTask({ threadIds: ['t1'], accountId: 'ac-1', starred: true });
    spyOn(TaskQueue, 'waitForPerformLocal').andReturn(Promise.resolve(task));
    UndoRedoStore._onQueue(task);
    expect(UndoRedoStore._undo.length).toBe(1);

    (Actions.queueTasks as any).andCallFake((tasks) => {
      queued.push(tasks);
      UndoRedoStore._onQueue(tasks);
    });
    UndoRedoStore.undo();
    await flushPromises();

    expect(requests).toEqual([]);
    expect(queued.length).toBe(1);
    expect(queued[0][0].starred).toBe(false);
    // The undo task isn't registered as undoable itself.
    expect(UndoRedoStore._undo.length).toBe(0);
    expect(UndoRedoStore._redo.length).toBe(1);
  });

  describe('Undo Send', function () {
    const draft = new Message({ id: 'd-1', accountId: 'ac-1', headerMessageId: 'h' } as any);
    const sendTask = (value: Record<string, unknown>) =>
      SyncbackMetadataTask.forSaving({
        pluginId: 'send-later',
        model: draft,
        value: { expiration: Date.now() + 5000, isUndoSend: true, actionKey: 'send', ...value },
        undoValue: { expiration: null, isUndoSend: true },
      });

    it("is the send's entry when DraftStore has its token", async function () {
      changed(entry('u5', { action: 'send', label: 'Sending 1 message', count: 1 }));
      // The entry's own toast waits for the block with the countdown.
      expect(UndoRedoStore.getMostRecent()).toBe(null);
      UndoRedoStore._onQueueUndoOnly(sendTask({ undoToken: 'u5' }));
      expect(UndoRedoStore._undo.length).toBe(1);
      expect(UndoRedoStore.getMostRecent().tasks.length).toBe(1);

      answers['undo.apply'] = { entry: entry('u5'), taskIds: [], cancelled: false };
      UndoRedoStore.undo();
      await flushPromises();
      expect(requests.map((r) => [r.method, r.params.token])).toEqual([['undo.apply', 'u5']]);
      expect(AppEnv.showErrorDialog).toHaveBeenCalled();
    });

    it("calls back a plugin send action's schedule without a token", async function () {
      answers['draft.cancelSend'] = { cancelled: true };
      UndoRedoStore._onQueueUndoOnly(sendTask({}));
      UndoRedoStore.undo();
      await flushPromises();

      expect(requests).toEqual([
        { method: 'draft.cancelSend', params: { accountId: 'ac-1', headerMessageId: 'h' } },
      ]);
      expect(queued.length).toBe(0);
      expect(AppEnv.showErrorDialog).not.toHaveBeenCalled();
    });

    it('keeps an entry that a composer window sends over the action bridge', function () {
      // What ActionBridge does: JSON across IPC, inflated with the model reviver.
      const [bridged] = JSON.parse(
        JSON.stringify([sendTask({ undoToken: 'u6' })]),
        Utils.modelTypesReviver
      );
      expect(bridged instanceof SyncbackMetadataTask).toBe(true);
      expect(bridged.canBeUndone).toBe(true);

      UndoRedoStore._onQueueUndoOnly(bridged);
      expect(UndoRedoStore._undo.length).toBe(1);
      expect((UndoRedoStore._undo[0] as any).token).toBe('u6');
    });
  });

  it('keeps canBeUndone on an event change inflated from JSON', function () {
    const local = new SyncbackEventTask({ accountId: 'ac-1', undoData: { id: 'e-1' } } as any);
    const [bridged] = JSON.parse(JSON.stringify([local]), Utils.modelTypesReviver);
    expect(bridged.canBeUndone).toBe(true);
  });
});
