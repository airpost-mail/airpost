import _ from 'underscore';
import BaseStore from './base';
import Rx from 'rx-lite';
import { Task } from '../tasks/task';
import DatabaseStore from '../database/store';
import * as Utils from '../models/utils';
import EngineEvents, { EngineNotification } from '../../engine/client/events';
import { taskDoneIn, type TaskJson } from '../../engine/protocol';

// TaskQueue is constructed as a module-level side effect below (`export default
// new TaskQueue()`), and its constructor calls `Rx.Observable.fromQuery`, which
// only exists once data/query/observables has patched it onto Rx. app-env.ts
// requires that module early, but this module can be required before that, so it
// imports it too.
import '../query/observables';

/*
Public: The TaskQueue is a Flux-compatible Store that manages a queue of {Task}
objects. Each {Task} represents an individual API action, like sending a draft
or marking a thread as "read". Tasks optimistically make changes to the app's
local cache and encapsulate logic for performing changes on the server, rolling
back in case of failure, and waiting on dependent tasks.

The TaskQueue is essential to offline mode in N1. It automatically pauses
when the user's internet connection is unavailable and resumes when online.

The task queue is persisted to disk, ensuring that tasks are executed later,
even if the user quits N1.

The queue is a live query of the Task table, which the engine keeps current. The main window,
which queues every task, also hears of each one directly: the `task.queue` answer is the task
after its local phase ('task-queued'), and the engine sends `task.done` when it finishes
(`task.queue {notify: true}`), so waitForPerformLocal and waitForPerformRemote resolve from
those without waiting for the queue to change.

The TaskQueue is only available in the app's main window. Rather than directly
queuing tasks, you should use the {Actions} to interact with the {TaskQueue}.
Tasks queued from secondary windows are serialized and sent to the application's
main window via IPC.

## Queueing a Task

```javascript
if (this._thread && this._thread.unread) {
  Actions.queueTask(new ChangeStarredTask({threads: [this._thread], starred: true}))
}
```

## Dequeueing a Task

```javascript
Actions.dequeueMatchingTask({
  type: 'DestroyCategoryTask',
  matching: {
    categoryId: 'bla',
  }
})
*/

class TaskQueue extends BaseStore {
  _queue: Task[] = [];
  _completed: Task[] = [];
  _currentSequentialId = Date.now();

  _waitingForLocal: Array<{ task: Task; resolve: (arg: any) => void }> = [];
  _waitingForRemote: Array<{ task: Task; resolve: (arg: any) => void }> = [];

  constructor() {
    super();
    Rx.Observable.fromQuery(DatabaseStore.findAll<Task>(Task)).subscribe(
      this._onQueueChangedDebounced
    );
    EngineEvents.on('task-queued', this._onTaskQueued);
    EngineEvents.on('notification', this._onNotification);
  }

  // The task after its local phase, from the task.queue answer.
  _onTaskQueued = (json: TaskJson) => {
    const id = (json as { id?: string }).id;
    if (!id || !this._waitingForLocal.some((w) => w.task.id === id)) return;
    const task = this._inflate(json);
    if (!task || !task.hasRunLocally()) return;
    this._waitingForLocal = this._waitingForLocal.filter(({ task: waiting, resolve }) => {
      if (waiting.id !== id) return true;
      resolve(task);
      return false;
    });
  };

  // `task.done` for a task this window queued.
  _onNotification = (n: EngineNotification) => {
    if (
      n.method !== 'task.done' ||
      this._waitingForRemote.length + this._waitingForLocal.length === 0
    ) {
      return;
    }
    const taskId = n.params && n.params.taskId;
    const done = typeof taskId === 'string' ? taskDoneIn(n, taskId) : null;
    if (!done || !done.task) return;
    const task = this._inflate(done.task);
    if (!task) return;
    const finish = ({ task: waiting, resolve }: { task: Task; resolve: (arg: any) => void }) => {
      if (waiting.id !== taskId) return true;
      resolve(task);
      return false;
    };
    this._waitingForLocal = this._waitingForLocal.filter(finish);
    // Complete or cancelled, which ends a remote wait as the queue's finished tasks do.
    this._waitingForRemote = this._waitingForRemote.filter(finish);
  };

  _inflate(json: unknown): Task | null {
    try {
      const model = Utils.convertToModel(json);
      return model instanceof Task ? model : null;
    } catch (err) {
      console.warn(`TaskQueue: could not read a task from the engine: ${err}`);
      return null;
    }
  }

  _onQueueChangedDebounced = _.throttle((tasks: Task[]) => {
    const finished = [Task.Status.Complete, Task.Status.Cancelled];
    this._queue = tasks.filter((t) => !finished.includes(t.status));
    this._completed = tasks.filter((t) => finished.includes(t.status));
    const all = [...this._queue, ...this._completed];

    this._waitingForLocal = this._waitingForLocal.filter(({ task, resolve }) => {
      const match = all.find((t) => task.id === t.id);
      if (match && match.hasRunLocally()) {
        resolve(match);
        return false;
      }
      return true;
    });

    this._waitingForRemote = this._waitingForRemote.filter(({ task, resolve }) => {
      const match = this._completed.find((t) => task.id === t.id);
      if (match) {
        resolve(match);
        return false;
      }
      return true;
    });

    this.trigger();
  }, 150);

  queue() {
    return this._queue;
  }

  completed() {
    return this._completed;
  }

  allTasks() {
    return [...this._queue, ...this._completed];
  }

  findTasks(
    typeOrClass: string | typeof Task,
    matching = {},
    { includeCompleted }: { includeCompleted?: boolean } = {}
  ) {
    const type = typeof typeOrClass === 'string' ? typeOrClass : typeOrClass.name;
    const tasks = includeCompleted ? [...this._queue, ...this._completed] : this._queue;

    const matches = tasks.filter((task) => {
      if (task.constructor.name !== type) {
        return false;
      }
      if (matching instanceof Function) {
        return matching(task);
      }
      return _.isMatch(task, matching);
    });

    return matches;
  }

  waitForPerformLocal = <T extends Task>(task: T) => {
    const upToDateTask = [...this._queue, ...this._completed].find((t) => t.id === task.id);
    if (upToDateTask && upToDateTask.hasRunLocally()) {
      return Promise.resolve(upToDateTask as T);
    }

    return new Promise<T>((resolve) => {
      this._waitingForLocal.push({ task, resolve });
    });
  };

  waitForPerformRemote = <T extends Task>(task: T) => {
    const upToDateTask = [...this._queue, ...this._completed].find((t) => t.id === task.id);
    if (upToDateTask && upToDateTask.status === Task.Status.Complete) {
      return Promise.resolve<T>(upToDateTask as T);
    }

    return new Promise<T>((resolve) => {
      this._waitingForRemote.push({ task, resolve });
    });
  };
}

export default new TaskQueue();
