import BaseStore from './base';
import * as Actions from '../actions';
import { Task } from '../tasks/task';
import { IntentTask, withThreadCount as withCount } from '../tasks/intent';
import TaskQueue from './task-queue';
import { localized } from '../../intl/localize';
import { cancelSend, undoSendTarget } from '../../engine/client/drafts';
import EngineEvents, { EngineNotification } from '../../engine/client/events';
import { OWN_CLIENT, undoApply, undoList, undoRecord, undoRedo } from '../../engine/client/intents';
import {
  Feature,
  Method,
  UndoApplyResult,
  UndoChangedParams,
  UndoEntry,
} from '../../engine/protocol';

/*
Undo and redo over the engine's undo log (`undo.*`). A change the app makes through an intent
(TaskFactory, the perspectives, the pickers, the sidebar), a snooze, a send inside its undo
window, or tasks queued with `canBeUndone` (handed to `undo.record` once their local phases
ran) is one entry of the log, which reaches every window as `undo.changed`. This store keeps
the app's own entries (`airpost-desktop`), so Cmd-Z never takes back what the CLI or an agent
did; `undo()` and `redo()` send the entry's token and the engine queues the inverse.

Two kinds of block stay in the app: an undo-only task (`Actions.queueUndoOnlyTask`) without an
engine token (a plugin's send action: draft.cancelSend, or the tasks' `createUndoTasks()`),
and tasks of classes the engine can't undo (a v1 plugin's own), reversed with
`createUndoTasks()` as before. An Undo Send block DraftStore queues with the token of the
send's entry is that entry, with the task the toast reads the countdown from.
*/

export interface UndoBlock {
  tasks: Task[];
  description: string;
  do: () => void;
  undo: () => void;
  redo?: () => void;
  /** The engine's undo log entry, as of its last `undo.changed`. */
  entry?: UndoEntry;
}

interface Block extends UndoBlock {
  token?: string;
}

// The task classes `undo.record` takes (airpost-tasks `undoable_class`).
const ENGINE_UNDOES = new Set([
  'ChangeUnreadTask',
  'ChangeStarredTask',
  'ChangeFolderTask',
  'ChangeLabelsTask',
  'SyncbackMetadataTask',
  'SyncbackEventTask',
  'ChangeContactGroupMembershipTask',
  'SyncbackContactGroupTask',
  'DestroyContactGroupTask',
]);

// The toast's wording of an entry, from its action and count.
export function describeEntry(entry: UndoEntry): string {
  const n = entry.count;
  switch (entry.action) {
    case 'archive':
      return withCount(localized(`Archived %@`), n);
    case 'trash':
      return withCount(localized(`Trashed %@`), n);
    case 'spam':
      return withCount(localized(`Marked %@ as Spam`), n);
    case 'notSpam':
      return withCount(localized(`Unmarked %@ as Spam`), n);
    case 'markRead':
    case 'markUnread':
    case 'categoryMarkRead': {
      const state = (
        entry.action === 'markUnread' ? localized('Unread') : localized('Read')
      ).toLocaleLowerCase();
      return n > 1
        ? localized(`Marked %@ threads as %@`, n, state)
        : localized(`Marked as %@`, state);
    }
    case 'star':
      return n > 1 ? localized('Starred %@ threads', n) : localized('Starred');
    case 'unstar':
      return n > 1 ? localized('Unstarred %@ threads', n) : localized('Unstarred');
    case 'snooze':
      return localized('Snoozed');
    case 'send':
      return localized('Sending soon...');
    default:
      // move, label and removeFromView: the caller describes them (`describe`); `tasks`
      // carries the label this store recorded.
      return entry.label;
  }
}

class UndoRedoStore extends BaseStore {
  _undo: Block[] = [];
  _redo: Block[] = [];
  _mostRecentBlock: Block | null = null;
  _queueingTasks = false;
  // What callers said of a token's change, for an entry that arrives after them.
  _descriptions = new Map<string, string>();

  constructor() {
    super();

    this.listenTo(Actions.queueTask, this._onQueue);
    this.listenTo(Actions.queueTasks, this._onQueue);
    this.listenTo(Actions.queueUndoOnlyTask, this._onQueueUndoOnly);
    EngineEvents.on('connected', this._load);
    EngineEvents.on('notification', this._onNotification);
  }

  // The app's entries as the log has them after each hello: the engine may have restarted
  // (an empty log) or this window reloaded. The blocks kept in the app stay.
  _load = (): void => {
    undoList().then(
      (entries) => {
        const local = (list: Block[]) => list.filter((b) => !b.token);
        const ours = entries.filter((e) => e.client === OWN_CLIENT).reverse();
        const blockOf = (e: UndoEntry) => {
          const known = this._find(e.token);
          if (known) known.entry = e;
          return known || this._blockFor(e);
        };
        this._undo = [
          ...local(this._undo),
          ...ours.filter((e) => e.state === 'done' && e.canUndo).map(blockOf),
        ];
        this._redo = [
          ...local(this._redo),
          ...ours.filter((e) => e.state === 'undone' && e.canRedo).map(blockOf),
        ];
        if (this._mostRecentBlock && !this._undo.includes(this._mostRecentBlock)) {
          this._mostRecentBlock = null;
        }
        this.trigger();
      },
      (err) => console.warn(`Could not read the undo log: ${err.message}`)
    );
  };

  _onNotification = ({ method, params }: EngineNotification): void => {
    if (method !== Method.UndoChanged) return;
    const { entry } = (params || {}) as UndoChangedParams;
    if (entry && entry.client === OWN_CLIENT) this._onEntry(entry);
  };

  _find(token: string): Block | undefined {
    return [...this._undo, ...this._redo].find((b) => b.token === token);
  }

  _remove(block: Block) {
    this._undo = this._undo.filter((b) => b !== block);
    this._redo = this._redo.filter((b) => b !== block);
    if (this._mostRecentBlock === block) this._mostRecentBlock = null;
  }

  _onEntry(entry: UndoEntry) {
    const block = this._find(entry.token);
    if (block) block.entry = entry;
    if (entry.state === 'gone') {
      if (block) this._remove(block);
    } else if (entry.state === 'undone') {
      // Undone here, or by another client with the token.
      if (block && this._undo.includes(block)) {
        this._remove(block);
        this._redo.push(block);
      }
    } else if (!block) {
      const fresh = this._blockFor(entry);
      this._redo = [];
      this._undo.push(fresh);
      // A send's toast is the undo-only block DraftStore queues with its token.
      if (entry.action !== 'send') this._mostRecentBlock = fresh;
    } else if (this._redo.includes(block)) {
      this._remove(block);
      this._undo.push(block);
    }
    this.trigger();
  }

  _blockFor(entry: UndoEntry): Block {
    const block: Block = {
      tasks: [],
      token: entry.token,
      entry,
      description: this._descriptions.get(entry.token) || describeEntry(entry),
      do: () => {},
      undo: () => this._apply(block, false),
      redo: () => this._apply(block, true),
    };
    return block;
  }

  _apply(block: Block, redo: boolean) {
    const send = !!block.entry && block.entry.action === 'send';
    (redo ? undoRedo : undoApply)(block.token).then(
      (result: UndoApplyResult) => {
        if (result.cancelled === false) {
          AppEnv.showErrorDialog(localized('The message was already sent.'));
        }
      },
      (err) => {
        if (send) {
          AppEnv.showErrorDialog(localized('Sorry, the send could not be undone. %@', err.message));
        } else {
          console.warn(`${redo ? 'Redo' : 'Undo'} skipped: ${err.message}`);
        }
        // The entry moved on (expired, failed, undone elsewhere): read the log again.
        this._load();
      }
    );
  }

  // What the toast says for `token`'s entry, from a caller that knows more than the entry's
  // action and count (the folder a move went to, the time a snooze ends).
  describe(token: string | null | undefined, description: string) {
    if (!token || !description) return;
    this._descriptions.set(token, description);
    const block = this._find(token);
    if (block && block.description !== description) {
      block.description = description;
      this.trigger();
    }
  }

  // An undo-only task (Undo Send) is registered here but never sent to the engine.
  _onQueueUndoOnly = (taskOrTasks: Task | Task[]): void => {
    const tasks = taskOrTasks instanceof Array ? taskOrTasks : [taskOrTasks];
    if (tasks.length === 0 || !tasks.every((t) => t.canBeUndone)) return;
    // Undo Send: the engine holds the send (or, for a plugin's send action, its schedule) and
    // calls it back, refusing once it has started. Not redoable: a redo would schedule it
    // again without the window's send transforms.
    const target = tasks.length === 1 ? undoSendTarget(tasks[0]) : null;
    const token: string | undefined = target ? (tasks[0] as any).value.undoToken : undefined;
    if (token) {
      const known = this._find(token);
      const block =
        known ||
        this._blockFor({ token, action: 'send', client: OWN_CLIENT, state: 'done' } as UndoEntry);
      block.tasks = tasks;
      block.description = tasks.map((t) => t.description()).join(', ');
      if (!known) {
        this._redo = [];
        this._undo.push(block);
      }
      this._mostRecentBlock = block;
      this.trigger();
      return;
    }
    if (target) {
      this._pushLocal(tasks, () =>
        cancelSend(target).then(
          (cancelled) => {
            if (!cancelled) AppEnv.showErrorDialog(localized('The message was already sent.'));
          },
          (err) =>
            AppEnv.showErrorDialog(
              localized('Sorry, the send could not be undone. %@', err.message)
            )
        )
      );
      return;
    }
    this._pushLocal(
      tasks,
      () => this._queueUndoTasks(tasks),
      () => this._requeue(tasks.map((t) => t.createIdenticalTask()))
    );
  };

  // Tasks queued with `canBeUndone` become one entry once their local phases ran. Intents
  // keep their own.
  _onQueue = (taskOrTasks: Task | Task[]): void => {
    if (this._queueingTasks) return;
    const tasks = (taskOrTasks instanceof Array ? taskOrTasks : [taskOrTasks]).filter(
      (t) => !(t instanceof IntentTask)
    );
    if (tasks.length === 0 || !tasks.every((t) => t.canBeUndone)) return;

    if (
      !EngineEvents.has(Feature.Undo) ||
      tasks.some((t) => !ENGINE_UNDOES.has(t.constructor.name))
    ) {
      this._pushLocal(
        tasks,
        () => this._queueUndoTasks(tasks),
        () => this._requeue(tasks.map((t) => t.createIdenticalTask()))
      );
      return;
    }
    const label = tasks.map((t) => t.description()).join(', ');
    Promise.all(tasks.map((t) => TaskQueue.waitForPerformLocal(t)))
      .then((done) => undoRecord(done, label))
      .catch((err) => console.warn(`No undo for ${label}: ${err.message}`));
  };

  _pushLocal(tasks: Task[], undo: () => void, redo?: () => void) {
    const block: Block = {
      tasks,
      description: tasks.map((t) => t.description()).join(', '),
      do: () => {},
      undo,
      redo,
    };
    this._redo = [];
    this._undo.push(block);
    this._mostRecentBlock = block;
    this.trigger();
  }

  // The inverse of tasks the engine doesn't keep, built from the versions their local phases
  // left (ChangeFolderTask's `undoPlacements`).
  _queueUndoTasks(tasks: Task[]) {
    Promise.all(tasks.map((t) => TaskQueue.waitForPerformLocal(t))).then((latest) => {
      let undoTasks: Task[];
      try {
        undoTasks = latest.flatMap((t) => t.createUndoTasks());
      } catch (err) {
        AppEnv.reportError(err);
        return;
      }
      if (undoTasks.length === 0) {
        console.warn('Undo skipped: no task could be built to reverse', tasks);
        return;
      }
      this._requeue(undoTasks);
    });
  }

  _requeue(tasks: Task[]) {
    this._queueingTasks = true;
    Actions.queueTasks(tasks);
    this._queueingTasks = false;
  }

  undo = (): void => {
    const block = this._undo.pop();
    if (!block) {
      return;
    }
    block.undo();

    this._mostRecentBlock = null;
    this._redo.push(block);
    this.trigger();
  };

  redo = (): void => {
    const block = this._redo.pop();
    if (!block) {
      return;
    }
    if (block.redo) {
      block.redo();
    } else {
      block.do();
    }
    this._mostRecentBlock = block;
    this._undo.push(block);
    this.trigger();
  };

  getMostRecent = (): UndoBlock | null => {
    return this._mostRecentBlock;
  };

  print(): void {
    console.log('Undo Stack');
    console.log(this._undo);
    console.log('Redo Stack');
    console.log(this._redo);
  }
}

export default new UndoRedoStore();
