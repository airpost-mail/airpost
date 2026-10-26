import DatabaseStore from '../database/store';
import { QueryRange } from './range';
import { MutableQueryResultSet } from './mutable-result-set';
import ModelQuery from './query';
import { Model } from '../models/model';
import { DatabaseChangeRecord } from '../database/change-record';
import { QueryResultSet } from './result-set';
import { attributeValue, ComparableValue } from '../models/attribute-access';
import type { LiveChange, LiveQuery, LiveRow, QueryWindow } from '@tern/client';
import EngineEvents from '../../engine/client/events';
import LiveQueries, { liveJSON, liveReadsMode } from './live';

// The live query window for a query's range: none (every row) for an infinite range.
function windowOf(range: QueryRange): QueryWindow | null {
  if (range.isInfinite()) return null;
  const window: QueryWindow = { offset: range.offset ?? 0 };
  if (range.limit !== null) window.limit = range.limit;
  return window;
}

type QuerySubscriptionResult<T extends Model> = QueryResultSet<T> | number | T | T[];
type QuerySubscriptionCallback<T extends Model> = (result: QuerySubscriptionResult<T>) => void;

/*
Public: a query's result, kept current. On the engine (`core.engine.reads: "engine"` and an
engine with live queries) it is a live query (`query.subscribe`, data/query/live.ts): the
engine decides membership and order and sends what changed, and the subscription only turns
rows into models. Otherwise it reads the query (SQLite, or the engine's `query`) and patches
the result from DatabaseStore's change records, evaluating the query's matchers on them.

A live subscription subscribes when its first callback is added and ends when its last one is
removed; reconnects subscribe again on their own.
*/
export class QuerySubscription<T extends Model> {
  _set: MutableQueryResultSet<T> | null = null;
  _callbacks: QuerySubscriptionCallback<T>[] = [];
  _lastResult: QuerySubscriptionResult<T> | undefined = undefined; // null is a valid result!
  _updateInFlight = false;
  _queuedChangeRecords: DatabaseChangeRecord<Model>[] = [];
  _queryVersion = 1;
  // Null for a MutableQuerySubscription made before its first replaceQuery (thread search).
  _query: ModelQuery<T> | ModelQuery<T[]> | null;
  _options: any;

  // How it reads (live.ts liveReadsMode); null until the first update decides.
  _mode: 'live' | 'reads' | null = null;
  _waitingForHello = false;
  _live: LiveQuery<LiveRow> | null = null;
  // The live query's params as JSON, to tell a new query from a new window.
  _liveKey: string | null = null;
  // Derived data the live query asks for on top of the query's own include (Thread summary).
  _liveInclude: string[] = [];
  // Models by the row they were made from: a row keeps its identity until a change replaces it.
  _liveModels = new WeakMap<object, T>();
  // The whole query's row count, from the live query.
  _liveTotal: number | null = null;

  constructor(
    query: ModelQuery<T> | ModelQuery<T[]> | null,
    options: {
      initialModels?: T[];
      emitResultSet?: boolean;
      updateOnSeparateThread?: boolean;
    } = {}
  ) {
    this._query = query;
    this._options = options;

    if (this._query) {
      if (this._query._count) {
        throw new Error('QuerySubscription::constructor - You cannot listen to count queries.');
      }

      this._query.finalize();

      if (this._options.initialModels) {
        this._set = new MutableQueryResultSet<T>();
        this._set.addModelsInRange(
          this._options.initialModels,
          new QueryRange({
            limit: this._options.initialModels.length,
            offset: 0,
          })
        );
        this._createResultAndTrigger();
      } else {
        this.update();
      }
    }
  }

  query = () => {
    return this._query;
  };

  addCallback = (callback: QuerySubscriptionCallback<T>) => {
    if (!(callback instanceof Function)) {
      throw new Error(`QuerySubscription:addCallback - expects a function, received ${callback}`);
    }
    this._callbacks.push(callback);
    if (this._lastResult !== undefined) {
      callback(this._lastResult);
    }
    if (this._mode === 'live' && !this._live && this._query) {
      this.update();
    }
  };

  hasCallback = (callback: QuerySubscriptionCallback<T>) => {
    return this._callbacks.indexOf(callback) !== -1;
  };

  removeCallback(callback: QuerySubscriptionCallback<T>) {
    if (!(callback instanceof Function)) {
      throw new Error(
        `QuerySubscription:removeCallback - expects a function, received ${callback}`
      );
    }
    this._callbacks = this._callbacks.filter((c) => c !== callback);
    if (this.callbackCount() === 0) {
      this._closeLive();
      this.onLastCallbackRemoved();
    }
  }

  onLastCallbackRemoved() {}

  callbackCount = () => {
    return this._callbacks.length;
  };

  applyChangeRecord = (record: DatabaseChangeRecord<Model>) => {
    if (this._mode === 'live' || this._waitingForHello) {
      return;
    }
    if (!this._query || record.objectClass !== this._query.objectClass()) {
      return;
    }
    if (record.objects.length === 0) {
      return;
    }

    this._queuedChangeRecords.push(record);
    if (!this._updateInFlight) {
      this._processChangeRecords();
    }
  };

  cancelPendingUpdate = () => {
    this._queryVersion += 1;
    this._updateInFlight = false;
  };

  // Reads the whole result again, dropping what the change records would have patched: the
  // pool calls it after the window reconnects to the engine, since deltas were missed.
  rerun() {
    if (!this._query) return;
    // A live query subscribes again after a reconnect itself; a changed `core.engine.reads`
    // moves the subscription to the other way of reading.
    const mode = liveReadsMode(this._query);
    if (mode !== 'wait' && this._mode !== null && mode !== this._mode) {
      this._closeLive();
      this._mode = mode;
    } else if (this._mode === 'live') {
      return;
    }
    this.cancelPendingUpdate();
    this._queuedChangeRecords = [];
    this._set = null;
    this.update();
  }

  // Scan through change records and apply them to the last result set.
  _processChangeRecords = () => {
    if (this._queuedChangeRecords.length === 0) {
      return;
    }
    const set = this._set;
    if (!set) {
      this.update();
      return;
    }

    let knownImpacts = 0;
    let unknownImpacts = 0;

    this._queuedChangeRecords.forEach((record) => {
      if (record.type === 'unpersist') {
        for (const item of record.objects as T[]) {
          const offset = set.offsetOfId(item.id);
          if (offset !== -1) {
            set.removeModelAtOffset(item, offset);
            unknownImpacts += 1;
          }
        }
      } else if (record.type === 'persist') {
        for (const item of record.objects as T[]) {
          const offset = set.offsetOfId(item.id);
          const itemIsInSet = offset !== -1;
          const itemShouldBeInSet = item.matches(this._requireQuery().matchers());

          if (itemIsInSet && !itemShouldBeInSet) {
            set.removeModelAtOffset(item, offset);
            unknownImpacts += 1;
          } else if (itemShouldBeInSet && !itemIsInSet) {
            set.updateModel(item);
            unknownImpacts += 1;
          } else if (itemIsInSet) {
            const oldItem = set.modelWithId(item.id);
            set.updateModel(item);

            if (this._itemSortOrderHasChanged(oldItem, item)) {
              unknownImpacts += 1;
            } else {
              knownImpacts += 1;
            }
          }
        }
        // If we're not at the top of the result set, we can't be sure whether an
        // item previously matched the set and doesn't anymore, impacting the items
        // in the query range. We need to refetch IDs to be sure our set === correct.
        if (
          (this._requireQuery().range().offset ?? 0) > 0 &&
          unknownImpacts + knownImpacts < record.objects.length
        ) {
          unknownImpacts += 1;
        }
      }
    });

    this._queuedChangeRecords = [];

    if (unknownImpacts > 0) {
      this.update({ mustRefetchEntireRange: true });
    } else if (knownImpacts > 0) {
      this._createResultAndTrigger();
    }
  };

  _itemSortOrderHasChanged(old: T, updated: T) {
    if (!old || !updated) return true;

    for (const descriptor of this._requireQuery().orderSortDescriptors()) {
      const oldSortValue = attributeValue(old, descriptor.attr.modelKey) as ComparableValue;
      const updatedSortValue = attributeValue(updated, descriptor.attr.modelKey) as ComparableValue;

      // http://stackoverflow.com/questions/4587060/determining-date-equality-in-javascript
      if (!(oldSortValue >= updatedSortValue && oldSortValue <= updatedSortValue)) {
        return true;
      }
    }
    return false;
  }

  update({ mustRefetchEntireRange }: { mustRefetchEntireRange?: boolean } = {}) {
    if (this._mode === null) {
      const mode = liveReadsMode(this._requireQuery());
      if (mode === 'wait') {
        this._waitForHello();
        return;
      }
      this._mode = mode;
    }
    if (this._mode === 'live') {
      this._updateLive();
      return;
    }

    this._updateInFlight = true;

    const desiredRange = this._requireQuery().range();
    const currentRange = this._set ? this._set.range() : null;
    const hasNonInfiniteRange =
      currentRange && !currentRange.isInfinite() && !desiredRange.isInfinite();

    // If we have a limited range, and changes don't require that we refetch
    // the entire range, just fetch the missing items. This is the path typically
    // used while scrolling.
    if (hasNonInfiniteRange && !mustRefetchEntireRange) {
      const missingRange = this._getMissingRange(desiredRange, currentRange);
      this._fetchRange(missingRange, {
        version: this._queryVersion,
        fetchEntireModels: true,
      });
    } else {
      const haveNoModels = !this._set || this._set.modelCacheCount() === 0;
      this._fetchRange(desiredRange, {
        version: this._queryVersion,
        fetchEntireModels: haveNoModels,
      });
    }
  }

  // Live queries

  // Adds derived data to the live query's rows (`include`), such as `summary` on Thread.
  includeInLive(...keys: string[]) {
    const added = keys.filter((k) => !this._liveInclude.includes(k));
    if (added.length === 0) return;
    this._liveInclude = [...this._liveInclude, ...added];
    if (this._live) this.update();
  }

  // The live query's whole row count; null when it isn't live or hasn't answered.
  liveTotal() {
    return this._live ? this._liveTotal : null;
  }

  // Reads once the first hello says how; nothing comes before it on either way.
  _waitForHello() {
    if (this._waitingForHello) return;
    this._waitingForHello = true;
    EngineEvents.once('connected', () => {
      this._waitingForHello = false;
      if (this._mode === null && this._query) this.update();
    });
  }

  _updateLive() {
    const query = this._requireQuery();
    const json = liveJSON(query);
    if (!json) {
      // A query with no JSON form reads the other way.
      this._closeLive();
      this._mode = 'reads';
      this.update();
      return;
    }
    const params = {
      ...json,
      include: [...json.include, ...this._liveInclude.filter((k) => !json.include.includes(k))],
    };
    const window = windowOf(query.range());
    const key = JSON.stringify({ ...params, limit: null, offset: null });
    if (!this._live) {
      if (this._callbacks.length === 0) return;
      const live = LiveQueries.query(params as any, window);
      this._live = live;
      this._liveKey = key;
      live.subscribe(this._onLiveRows);
      live.onError((err) => {
        if (this._live === live) this._onReadFailed(err as Error);
      });
      return;
    }
    if (key !== this._liveKey) {
      this._liveKey = key;
      this._live.setQuery(params as any, window);
    } else {
      this._live.setWindow(window);
    }
  }

  _closeLive() {
    if (!this._live) return;
    this._live.close();
    this._live = null;
    this._liveKey = null;
    this._liveTotal = null;
  }

  _onLiveRows = (live: LiveQuery<LiveRow>, _change: LiveChange) => {
    if (live !== this._live) return;
    const models: T[] = [];
    for (const row of live.rows) {
      let model = this._liveModels.get(row);
      if (!model) {
        const inflated = this._inflateLiveRow(row);
        if (!inflated) continue;
        model = inflated;
        this._liveModels.set(row, model);
      }
      models.push(model);
    }
    const set = new MutableQueryResultSet<T>();
    set.addModelsInRange(models, new QueryRange({ offset: live.offset, limit: models.length }));
    set._live = true;
    set._liveWindow = JSON.stringify(live.rowsWindow);
    this._set = set;
    this._liveTotal = live.total;
    this._updateInFlight = false;
    this._createResultAndTrigger();
  };

  // A row as a model; null for one the engine gave as only its id (gone between its reads,
  // which the next change removes) or that doesn't inflate.
  _inflateLiveRow(row: LiveRow): T | null {
    if (!row.__cls) return null;
    try {
      const [model] = this._requireQuery().inflateEngineResult({ rows: [row] }) as T[];
      return model || null;
    } catch (err) {
      console.warn(
        `QuerySubscription: could not read a live ${this._requireQuery().objectClass()} row: ${err}`
      );
      return null;
    }
  }

  _getMissingRange = (desiredRange: QueryRange, currentRange: QueryRange | null) => {
    if (currentRange && !currentRange.isInfinite() && !desiredRange.isInfinite()) {
      const ranges = QueryRange.rangesBySubtracting(desiredRange, currentRange);
      return ranges.length === 1 ? ranges[0] : desiredRange;
    }
    return desiredRange;
  };

  _getQueryForRange = (range: QueryRange, fetchEntireModels: boolean) => {
    const query = this._requireQuery();
    let rangeQuery: ModelQuery<T> | ModelQuery<T[]> | null = null;
    if (!range.isInfinite()) {
      rangeQuery = rangeQuery || query.clone();
      rangeQuery.offset(range.offset).limit(range.limit);
    }
    if (!fetchEntireModels) {
      rangeQuery = rangeQuery || query.clone();
      rangeQuery.idsOnly();
    }
    rangeQuery = rangeQuery || query;
    return rangeQuery;
  };

  _fetchRange(
    range: QueryRange,
    { version, fetchEntireModels }: { version: number; fetchEntireModels: boolean }
  ) {
    const rangeQuery = this._getQueryForRange(range, fetchEntireModels);
    const haveModels = this._set && this._set.modelCacheCount() > 0;

    if (haveModels && this._options.updateOnSeparateThread) {
      rangeQuery.background();
    }

    // A read that fails (the engine unavailable) keeps the last result; the change records wait
    // for the next update or rerun().
    const readFailed = (err: Error) => {
      if (this._queryVersion !== version) return;
      this._updateInFlight = false;
      this._onReadFailed(err);
    };

    DatabaseStore.run<T[] | string[]>(rangeQuery, { format: false }).then(async (results) => {
      if (this._queryVersion !== version) {
        return;
      }

      // Use the actual results length (not the requested range limit) to check
      // contiguity. If fewer items were returned than requested (end of list,
      // or items deleted since the scroll position was computed), the actual
      // data may not reach the existing set — treating the requested range as
      // contiguous would pass the check but then throw inside addIdsInRange.
      // For infinite ranges keep the original `range` so isContiguousWith can
      // short-circuit with `return true` as it always did for infinite ranges.
      const contiguityRange = range.isInfinite()
        ? range
        : new QueryRange({ offset: range.offset, limit: results.length });
      if (this._set && !this._set.range().isContiguousWith(contiguityRange)) {
        this._set = null;
      }
      const set = this._set || new MutableQueryResultSet<T>();
      this._set = set;

      if (fetchEntireModels) {
        set.addModelsInRange(results as T[], range);
      } else {
        set.addIdsInRange(results as string[], range);
      }

      set.clipToRange(this._requireQuery().range());

      // todo: this is returning fewer objects because they're being deleted immediately after being saved
      let models: T[];
      try {
        models = await this._fetchMissingModels();
      } catch (err) {
        readFailed(err as Error);
        return;
      }
      if (this._queryVersion !== version) {
        return;
      }
      for (const m of models) {
        set.updateModel(m);
      }
      this._createResultAndTrigger();
    }, readFailed);
  }

  // The query, which reads and change records only reach once the subscription has one.
  _requireQuery() {
    if (!this._query) {
      throw new Error('QuerySubscription: there is no query yet');
    }
    return this._query;
  }

  // The result set, which the callers below only reach once a read or the initial models made it.
  _requireSet() {
    if (!this._set) {
      throw new Error('QuerySubscription: there is no result set yet');
    }
    return this._set;
  }

  async _fetchMissingModels() {
    const set = this._requireSet();
    const missingIds = set.ids().filter((id) => !set.modelWithId(id));
    if (missingIds.length === 0) {
      return [];
    }
    return DatabaseStore.findAll<T>(this._requireQuery()._klass, { id: missingIds });
  }

  // A read that failed (the engine unavailable, or refusing the query).
  _onReadFailed(err: Error) {
    console.warn(`QuerySubscription: could not read ${this._requireQuery().objectClass()}: ${err}`);
  }

  _createResultAndTrigger() {
    const set = this._requireSet();
    const allCompleteModels = set.isComplete();

    const d: { [id: string]: number } = {};
    const a = set.ids();
    for (const ai of a) d[ai] = 1;
    const allUniqueIds = Object.keys(d).length === a.length;

    let error: Error | null = null;
    if (!allCompleteModels) {
      error = new Error('QuerySubscription: Applied all changes and result set is missing models.');
    }
    if (!allUniqueIds) {
      error = new Error(
        'QuerySubscription: Applied all changes and result set contains duplicate IDs.'
      );
    }

    if (error) {
      console.warn(error);
      // this._set = null;
      // this.update();
      // return;
    }

    const query = this._requireQuery();
    if (this._options.emitResultSet) {
      set.setQuery(query);
      this._lastResult = set.immutableClone();
    } else {
      const models = set.models();
      this._lastResult = query.formatResult(models) as QuerySubscriptionResult<T>;
    }

    const result = this._lastResult;
    this._callbacks.forEach((callback) => callback(result));

    // process any additional change records that have arrived
    if (this._updateInFlight) {
      this._updateInFlight = false;
      this._processChangeRecords();
    }
  }
}
