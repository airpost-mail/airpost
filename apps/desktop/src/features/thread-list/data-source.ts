/*
The thread list's rows: the perspective's thread subscription, with what each row shows of the
thread's messages.

On a live query the subscription asks the engine for `include: ["summary"]`, so each row
carries it (summary.ts) and no messages are read. Rows still have `__messages` for v1 plugins
(a ThreadListIcon reading the last message's metadata): a getter that loads the messages of
the threads it is read for, in one live query, and gives an empty list until they are in.

Otherwise (`core.engine.reads: "sqlite"`, or an engine without live queries) the list joins
every visible thread's messages: one read for the threads that came into view, then one
subscription per thread.
*/
import Rx from 'rx-lite';
import { ObservableListDataSource } from '../../components/list/observable-data-source';
import DatabaseStore from '../../data/database/store';
import { Thread } from '../../data/models/thread';
import { Message } from '../../data/models/message';
import { QueryResultSet } from '../../data/query/result-set';
import { QuerySubscription } from '../../data/query/subscription';
import { MutableQuerySubscription } from '../../data/query/mutable-subscription';
import { ThreadWithMessagesMetadata } from './types';

// The most threads whose messages the `__messages` getter keeps loaded.
const MAX_LAZY_THREADS = 300;
let lazySubscriptions = 0;

// The messages `__messages` was read for, in one subscription, rebuilt as the threads change.
export class LazyThreadMessages {
  _wanted = new Set<string>();
  _byThread = new Map<string, Message[]>();
  _subscription: MutableQuerySubscription<Message> | null = null;
  _unsubscribe: (() => void) | null = null;
  _scheduled = false;
  _key = `thread-list-messages-${++lazySubscriptions}`;
  $changes = new Rx.BehaviorSubject<number>(0);

  messagesFor(threadId: string): Message[] {
    if (!this._wanted.has(threadId)) {
      this._wanted.add(threadId);
      this._schedule();
    }
    return this._byThread.get(threadId) || [];
  }

  // Forgets the threads that left the list.
  keepOnly(threadIds: string[]) {
    const keep = new Set(threadIds);
    let changed = false;
    for (const id of [...this._wanted]) {
      if (!keep.has(id)) {
        this._wanted.delete(id);
        this._byThread.delete(id);
        changed = true;
      }
    }
    if (changed && this._subscription) this._schedule();
  }

  cleanup() {
    if (this._unsubscribe) this._unsubscribe();
    this._unsubscribe = null;
    this._subscription = null;
    this.$changes.onCompleted();
  }

  _schedule() {
    if (this._scheduled) return;
    this._scheduled = true;
    setTimeout(() => {
      this._scheduled = false;
      this._resubscribe();
    }, 0);
  }

  _resubscribe() {
    const ids = [...this._wanted].slice(-MAX_LAZY_THREADS);
    if (ids.length === 0) return;
    const query = DatabaseStore.findAll<Message>(Message, { threadId: ids });
    if (this._subscription) {
      this._subscription.replaceQuery(query);
      return;
    }
    const subscription = new MutableQuerySubscription<Message>(query);
    this._subscription = subscription;
    const disposable = (
      Rx.Observable.fromNamedQuerySubscription(this._key, subscription) as Rx.Observable<Message[]>
    ).subscribe((messages) => this._onMessages(messages));
    this._unsubscribe = () => disposable.dispose();
  }

  _onMessages(messages: Message[]) {
    const byThread = new Map<string, Message[]>();
    for (const id of this._wanted) byThread.set(id, []);
    for (const message of messages) {
      if (message.isHidden()) continue;
      const list = byThread.get(message.threadId);
      if (list) list.push(message);
    }
    this._byThread = byThread;
    this.$changes.onNext(this.$changes.getValue() + 1);
  }
}

// A row of a live list: the thread with its summary, and `__messages` for v1 plugins.
function withLazyMessages(thread: Thread, lazy: LazyThreadMessages): ThreadWithMessagesMetadata {
  const clone = new Thread(thread) as ThreadWithMessagesMetadata;
  Object.defineProperty(clone, '__messages', {
    get: () => lazy.messagesFor(clone.id),
    enumerable: false,
    configurable: true,
  });
  return clone;
}

// The rows carry the engine's summary.
function carriesSummary(resultSet: QueryResultSet<Thread>) {
  return resultSet._live && resultSet.models().every((t) => !!t && t.summary !== undefined);
}

const _summaryRows = (
  $threadsResultSet: Rx.Observable<QueryResultSet<Thread>>,
  lazy: LazyThreadMessages
) =>
  $threadsResultSet.combineLatest(lazy.$changes, (threadsResultSet) => {
    lazy.keepOnly(threadsResultSet.ids());
    const rows: { [id: string]: ThreadWithMessagesMetadata } = {};
    for (const thread of threadsResultSet.models()) {
      rows[thread.id] = withLazyMessages(thread, lazy);
    }
    return QueryResultSet.setByApplyingModels(threadsResultSet, rows);
  });

const _observableForThreadMessages = (id: string, initialModels: Message[]) => {
  const subscription = new QuerySubscription<Message>(
    DatabaseStore.findAll<Message>(Message, { threadId: id }),
    {
      initialModels: initialModels,
      emitResultSet: true,
    }
  );
  // emitResultSet: the subscription emits result sets, not model arrays.
  return Rx.Observable.fromNamedQuerySubscription(`message-${id}`, subscription) as Rx.Observable<
    QueryResultSet<Message>
  >;
};

const _flatMapJoiningMessages = ($threadsResultSet: Rx.Observable<QueryResultSet<Thread>>) => {
  // DatabaseView leverages `QuerySubscription` for threads /and/ for the
  // messages on each thread, which are passed to out as `thread.__messages`.
  let $messagesResultSets: { [threadId: string]: Rx.Observable<QueryResultSet<Message>> } = {};

  // 2. when we receive a set of threads, we check to see if we have message
  //    observables for each thread. If threads have been added to the result set,
  //    we make a single database query and load /all/ the message metadata for
  //    the new threads at once. (This is a performance optimization -it's about
  //    ~80msec faster than making 100 queries for 100 new thread ids separately.)
  return (
    $threadsResultSet
      .flatMapLatest((threadsResultSet) => {
        const missingIds = threadsResultSet.ids().filter((id) => !$messagesResultSets[id]);
        let promise: Promise<[QueryResultSet<Thread>, Message[]]>;
        if (missingIds.length === 0) {
          promise = Promise.resolve([threadsResultSet, [] as Message[]]);
        } else {
          promise = DatabaseStore.findAll<Message>(Message, { threadId: missingIds }).then(
            (messages): [QueryResultSet<Thread>, Message[]] => [threadsResultSet, messages]
          );
        }
        return Rx.Observable.fromPromise(promise);
      })
      // 3. when that finishes, we group the loaded messsages by threadId and create
      //    the missing observables. Creating a query subscription would normally load
      //    an initial result set. To avoid that, we just hand new subscriptions the
      //    results we loaded in #2.
      .flatMapLatest(([threadsResultSet, messagesForNewThreads]) => {
        const messagesGrouped: { [threadId: string]: Message[] } = {};
        for (const message of messagesForNewThreads) {
          if (messagesGrouped[message.threadId] == null) {
            messagesGrouped[message.threadId] = [];
          }
          messagesGrouped[message.threadId].push(message);
        }

        const oldSets = $messagesResultSets;
        $messagesResultSets = {};

        const sets = threadsResultSet.ids().map((id) => {
          $messagesResultSets[id] =
            oldSets[id] || _observableForThreadMessages(id, messagesGrouped[id]);
          return $messagesResultSets[id];
        });
        // 4. We use `combineLatest` to merge the message observables into a single
        //    stream (like Promise.all).  When /any/ of them emit a new result set, we
        //    trigger.
        return Rx.Observable.from([threadsResultSet]).combineLatest(
          sets,
          (threads, ...messageSets): [QueryResultSet<Thread>, QueryResultSet<Message>[]] => [
            threads,
            messageSets,
          ]
        );
      })
      .flatMapLatest(([threadsResultSet, messagesResultSets]) => {
        const threadsWithMessages: { [id: string]: ThreadWithMessagesMetadata } = {};
        threadsResultSet.models().forEach((thread, idx) => {
          const clone = new Thread(thread) as ThreadWithMessagesMetadata;
          clone.__messages = messagesResultSets[idx] ? messagesResultSets[idx].models() : [];
          clone.__messages = clone.__messages.filter((m) => !m.isHidden());
          threadsWithMessages[clone.id] = clone;
        });

        return Rx.Observable.from([
          QueryResultSet.setByApplyingModels(threadsResultSet, threadsWithMessages),
        ]);
      })
  );
};

class ThreadListDataSource extends ObservableListDataSource {
  _lazyMessages: LazyThreadMessages;

  constructor(subscription: MutableQuerySubscription<Thread>) {
    subscription.includeInLive('summary');
    // The subscription emits result sets (emitResultSet), not model arrays.
    const $resultSetObservable = (
      Rx.Observable.fromNamedQuerySubscription('thread-list', subscription) as Rx.Observable<
        QueryResultSet<Thread>
      >
    ).share();
    const lazy = new LazyThreadMessages();
    // One route per subscription in practice; a join still out when rows with summaries come
    // is dropped.
    let summaries = false;
    const $marked = $resultSetObservable.do((set) => {
      summaries = carriesSummary(set);
    });
    const $joined = _flatMapJoiningMessages($marked.filter((set) => !carriesSummary(set))).filter(
      () => !summaries
    );
    const $summarized = _summaryRows(
      $marked.filter((set) => carriesSummary(set)),
      lazy
    );
    super(Rx.Observable.merge($joined, $summarized), subscription.replaceRange.bind(subscription));
    this._lazyMessages = lazy;
  }

  cleanup() {
    this._lazyMessages.cleanup();
    return super.cleanup();
  }
}

export default ThreadListDataSource;
