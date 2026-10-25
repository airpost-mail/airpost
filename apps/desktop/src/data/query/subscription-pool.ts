/* eslint global-require: 0 */
import { QuerySubscription } from './subscription';
import { DatabaseChangeRecord } from '../database/change-record';
import ModelQuery from './query';
import { Model } from '../models/model';

/*
Public: The QuerySubscriptionPool maintains a list of all of the query
subscriptions in the app. In the future, this class will monitor performance,
merge equivalent subscriptions, etc.
*/
class QuerySubscriptionPool {
  _subscriptions: { [key: string]: QuerySubscription<Model> } = {};
  _cleanupChecks: string[] = [];

  constructor() {
    this._setup();
  }

  add(query: ModelQuery<any>, callback: ((result: any) => void) & { _registrationPoint?: string }) {
    if (AppEnv.inDevMode()) {
      callback._registrationPoint = this._formatRegistrationPoint(new Error().stack || '');
    }

    const key = this._keyForQuery(query);
    let subscription = this._subscriptions[key];
    if (!subscription) {
      subscription = new QuerySubscription(query);
      this._subscriptions[key] = subscription;
    }

    subscription.addCallback(callback);
    return () => {
      subscription.removeCallback(callback);
      this._scheduleCleanupCheckForSubscription(key);
    };
  }

  addPrivateSubscription(
    key: string,
    subscription: QuerySubscription<any>,
    callback: ((result: any) => void) & { _registrationPoint?: string }
  ) {
    this._subscriptions[key] = subscription;
    subscription.addCallback(callback);
    return () => {
      subscription.removeCallback(callback);
      this._scheduleCleanupCheckForSubscription(key);
    };
  }

  printSubscriptions() {
    if (!AppEnv.inDevMode()) {
      console.log('printSubscriptions is only available in developer mode.');
      return;
    }

    for (const key of Object.keys(this._subscriptions)) {
      const subscription = this._subscriptions[key];
      console.log(key);
      console.group();
      for (const callback of subscription._callbacks as { _registrationPoint?: string }[]) {
        console.log(`${callback._registrationPoint}`);
      }
      console.groupEnd();
    }
  }

  _scheduleCleanupCheckForSubscription(key: string) {
    // We unlisten / relisten to lots of subscriptions and setTimeout is actually
    // /not/ that fast. Create one timeout for all checks, not one for each.
    if (this._cleanupChecks.length === 0) {
      setTimeout(() => this._runCleanupChecks(), 1);
    }
    this._cleanupChecks.push(key);
  }

  _runCleanupChecks() {
    for (const key of this._cleanupChecks) {
      const subscription = this._subscriptions[key];
      if (subscription && subscription.callbackCount() === 0) {
        delete this._subscriptions[key];
      }
    }
    this._cleanupChecks = [];
  }

  _formatRegistrationPoint(stackString: string) {
    const stack = stackString.split('\n');
    let ii = 0;
    let seenRx = false;
    while (ii < stack.length) {
      const hasRx = stack[ii].indexOf('rx.lite') !== -1;
      seenRx = seenRx || hasRx;
      if (seenRx === true && !hasRx) {
        break;
      }
      ii += 1;
    }

    return stack.slice(ii, ii + 4).join('\n');
  }

  // The query as the engine reads it; one with no JSON form keys on its SQL.
  _keyForQuery<T extends Model | Model[]>(query: ModelQuery<T>) {
    try {
      return JSON.stringify(query.toJSON());
    } catch (err) {
      return query.sql();
    }
  }

  _setup() {
    const DatabaseStore: typeof import('../database/store').default =
      require('../database/store').default;
    DatabaseStore.listen(this._onChange);
    require('../database/engine-reads').default.on('resync', this._rerunAll);
  }

  // After a reconnect: the deltas sent while the window had no connection are gone.
  _rerunAll = () => {
    for (const key of Object.keys(this._subscriptions)) {
      const subscription = this._subscriptions[key];
      if (subscription.callbackCount() > 0) subscription.rerun();
    }
  };

  _onChange = (record: DatabaseChangeRecord<Model>) => {
    for (const key of Object.keys(this._subscriptions)) {
      const subscription = this._subscriptions[key];
      subscription.applyChangeRecord(record);
    }
  };
}

const pool = new QuerySubscriptionPool();
export default pool;
