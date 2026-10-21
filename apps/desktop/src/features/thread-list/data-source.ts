import Rx from 'rx-lite';
import { ObservableListDataSource } from '../../components/list/observable-data-source';
import DatabaseStore from '../../data/database/store';
import { Thread } from '../../data/models/thread';
import { Message } from '../../data/models/message';
import { QueryResultSet } from '../../data/query/result-set';
import { QuerySubscription } from '../../data/query/subscription';
import { MutableQuerySubscription } from '../../data/query/mutable-subscription';
import { ThreadWithMessagesMetadata } from './types';

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
  constructor(subscription: MutableQuerySubscription<Thread>) {
    // The subscription emits result sets (emitResultSet), not model arrays.
    const $resultSetObservable = Rx.Observable.fromNamedQuerySubscription(
      'thread-list',
      subscription
    ) as Rx.Observable<QueryResultSet<Thread>>;
    super(
      _flatMapJoiningMessages($resultSetObservable),
      subscription.replaceRange.bind(subscription)
    );
  }
}

export default ThreadListDataSource;
