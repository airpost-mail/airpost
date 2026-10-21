import { QueryRange } from './range';
import { Model } from '../models/model';
import ModelQuery from './query';

/*
Public: Instances of QueryResultSet hold a set of models retrieved
from the database at a given offset.

Complete vs Incomplete:

QueryResultSet keeps an array of item ids and a lookup table of models.
The lookup table may be incomplete if the QuerySubscription isn't finished
preparing results. You can use `isComplete` to determine whether the set
has every model.

Offset vs Index:

To avoid confusion, "index" refers to an item's position in an
array, and "offset" refers to it's position in the query result set. For example,
an item might be at index 20 in the _ids array, but at offset 120 in the result.
*/
export class QueryResultSet<T extends Model> {
  // Null until the set holds a range.
  _offset: number | null;
  _query: ModelQuery<T> | ModelQuery<T[]> | null;
  // Null when it needs rebuilding (see offsetOfId).
  _idToIndexHash: { [id: string]: number } | null;
  _modelsHash: { [id: string]: T };
  _ids: string[] = [];

  static setByApplyingModels<T extends Model>(set: QueryResultSet<T>, models: { [id: string]: T }) {
    if (models instanceof Array) {
      throw new Error('setByApplyingModels: A hash of models is required.');
    }
    const out = set.clone();
    out._modelsHash = models;
    out._idToIndexHash = null;
    return out;
  }

  constructor(other: Partial<QueryResultSet<T>> = {}) {
    this._offset = other._offset !== undefined ? other._offset : null;
    this._query = other._query !== undefined ? other._query : null;
    this._idToIndexHash = other._idToIndexHash !== undefined ? other._idToIndexHash : null;
    // Clone, since the others may be frozen
    this._modelsHash = Object.assign({}, other._modelsHash || {});
    this._ids = [...(other._ids || [])];
  }

  clone(): this {
    const Klass = this.constructor as typeof QueryResultSet;
    return new Klass<T>({
      _ids: [...this._ids],
      _modelsHash: { ...this._modelsHash },
      _idToIndexHash: { ...this._idToIndexHash },
      _query: this._query,
      _offset: this._offset,
    }) as this;
  }

  isComplete() {
    return this._ids.every((id) => !!this._modelsHash[id]);
  }

  range() {
    return new QueryRange({ offset: this._offset, limit: this._ids.length });
  }

  query() {
    return this._query;
  }

  count() {
    return this._ids.length;
  }

  empty() {
    return this.count() === 0;
  }

  ids() {
    return this._ids;
  }

  idAtOffset(offset: number) {
    return this._ids[offset - (this._offset ?? 0)];
  }

  models() {
    return this._ids.map((id) => this._modelsHash[id]);
  }

  modelCacheCount() {
    return Object.keys(this._modelsHash).length;
  }

  modelAtOffset(offset: number) {
    if (!Number.isInteger(offset)) {
      throw new Error(
        'QueryResultSet.modelAtOffset() takes a numeric index. Maybe you meant modelWithId()?'
      );
    }
    return this._modelsHash[this._ids[offset - (this._offset ?? 0)]];
  }

  modelWithId(id: string): T {
    return this._modelsHash[id];
  }

  buildIdToIndexHash() {
    const hash: { [id: string]: number } = {};
    this._ids.forEach((id, idx) => {
      hash[id] = idx;
    });
    this._idToIndexHash = hash;
    return hash;
  }

  offsetOfId(id: string) {
    const hash = this._idToIndexHash === null ? this.buildIdToIndexHash() : this._idToIndexHash;

    if (hash[id] === undefined) {
      return -1;
    }
    return hash[id] + (this._offset ?? 0);
  }
}
