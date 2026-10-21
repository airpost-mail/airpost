import BaseStore from './base';
import { SendDraftTask } from '../tasks/send-draft';
import { SyncbackDraftTask } from '../tasks/syncback-draft';
import TaskQueue from './task-queue';

// The engine doesn't report send progress today, so `progress` is undefined at runtime.
export type OutboxTask = (SendDraftTask | SyncbackDraftTask) & {
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
    return this._tasks.filter((task) => (task.draft?.accountId ?? task.accountId) === accountId);
  }
}

const store = new OutboxStore();
export default store;
