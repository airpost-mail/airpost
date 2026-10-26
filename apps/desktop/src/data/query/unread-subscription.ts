import * as Actions from '../actions';
import { MutableQuerySubscription } from './mutable-subscription';
import DatabaseStore from '../database/store';
import { Matcher } from '../attributes/matcher';
import { Thread } from '../models/thread';
import { Task } from '../tasks/task';
import { ChangeUnreadTask } from '../tasks/change-unread';
import { ChangeLabelsTask } from '../tasks/change-labels';
import { ChangeFolderTask } from '../tasks/change-folder';
import { IntentTask } from '../tasks/intent';
import { Method } from '../../engine/protocol';

// The intents that take threads out of a list.
const MOVES: string[] = [
  Method.ThreadsArchive,
  Method.ThreadsTrash,
  Method.ThreadsSpam,
  Method.ThreadsNotSpam,
  Method.ThreadsMove,
  Method.ThreadsLabel,
  Method.ThreadsRemoveFromView,
];

const isMarkRead = (task: Task): task is IntentTask =>
  task instanceof IntentTask && task.intent === Method.ThreadsMarkRead && !task.params.unread;

// The Unread view keeps a thread the user reads until they leave the view: the threads read
// while this subscription lives are whitelisted. Owning the list here ends it however the
// view is left, and nesting it under the category filter keeps threads read elsewhere out.
// One categories matcher keeps the query on ModelQuery's ThreadCategory subselect.
const buildQuery = (categoryIds: string[], recentlyReadIds: string[]) => {
  const unread = new Matcher.And([
    Thread.attributes.unread.equal(true),
    Thread.attributes.inAllMail.equal(true),
  ]);
  return DatabaseStore.findAll<Thread>(Thread)
    .where([
      Thread.attributes.categories.containsAny(categoryIds),
      recentlyReadIds.length
        ? new Matcher.Or([unread, Thread.attributes.id.in(recentlyReadIds)])
        : unread,
    ])
    .limit(0);
};

export default class UnreadQuerySubscription extends MutableQuerySubscription<Thread> {
  _categoryIds: string[];
  _recentlyReadIds: string[] = [];
  _unlisteners: Array<() => void>;

  constructor(categoryIds: string[]) {
    super(buildQuery(categoryIds, []), { emitResultSet: true });
    this._categoryIds = categoryIds;
    this._unlisteners = [
      Actions.queueTask.listen((task: Task) => this.onTasksQueued([task])),
      Actions.queueTasks.listen((tasks: Task[]) => this.onTasksQueued(tasks)),
    ];
  }

  onTasksQueued = (tasks: Task[]) => {
    let ids = this._recentlyReadIds;
    for (const task of tasks) {
      if (task instanceof ChangeUnreadTask || isMarkRead(task)) {
        ids = ids.concat(task.threadIds);
      } else if (
        task instanceof ChangeLabelsTask ||
        task instanceof ChangeFolderTask ||
        (task instanceof IntentTask && MOVES.includes(task.intent))
      ) {
        const moved = task.threadIds;
        ids = ids.filter((id) => !moved.includes(id));
      }
    }
    if (ids === this._recentlyReadIds) {
      return;
    }
    this._recentlyReadIds = ids;
    // Reading change records, swapped without a refetch: the thread's own delta is matched
    // against the new query. A live query subscribes to the new one, which the engine answers
    // with the whitelisted threads still in it.
    const { limit, offset } = this._requireQuery().range();
    this._query = buildQuery(this._categoryIds, ids).limit(limit).offset(offset);
    if (this._mode === 'live') this.update();
  };

  onLastCallbackRemoved() {
    this._unlisteners.forEach((unlisten) => unlisten());
  }
}
