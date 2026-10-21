import BaseStore from './base';
import { SendDraftTask } from '../tasks/send-draft';
import { SyncbackDraftTask } from '../tasks/syncback-draft';
import TaskQueue from './task-queue';

// Readers use draftAccountId and progress, which neither task type sets today, so both are
// always undefined at runtime.
export type OutboxTask = (SendDraftTask | SyncbackDraftTask) & {
  draftAccountId?: string;
  progress?: number;
};

class OutboxStore extends BaseStore {
  _tasks: OutboxTask[] = [];

  constructor() {
    super();
    this.listenTo(TaskQueue, this._populate);
    this._populate();
  }

  _populate() {
    const nextTasks = TaskQueue.queue().filter(
      (task): task is SendDraftTask | SyncbackDraftTask =>
        task instanceof SendDraftTask || task instanceof SyncbackDraftTask
    );
    if (this._tasks.length === 0 && nextTasks.length === 0) {
      return;
    }
    this._tasks = nextTasks;
    this.trigger();
  }

  itemsForAccount(accountId: string) {
    // No task has a draftAccountId, so this matches nothing and the Drafts perspective's
    // count stays 0. Kept as it was; matching on the draft's account would start showing it.
    return this._tasks.filter((task) => task.draftAccountId === accountId);
  }
}

const store = new OutboxStore();
export default store;
