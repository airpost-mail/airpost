import _ from 'underscore';
import BaseStore from './base';
import EngineEvents, {
  EngineEvents as EngineEventsType,
  ThreadCountsDelta,
} from '../../engine/client/events';
import { CountsGetResult, Method } from '../../engine/protocol';

type Counts = { [categoryId: string]: { unread: number; total: number } };

/*
Unread and total thread counts per folder and label. The engine answers `counts.get` once per
connection and sends every change after it as a ThreadCounts delta. Until the first hello the
counts are empty; when the connection drops they stay as they were.
*/
export class ThreadCountsStore extends BaseStore {
  _counts: Counts = {};
  _engine: EngineEventsType;
  // A hello answered and the deltas apply.
  _connected = false;
  // Set while counts.get is out. Deltas that arrive meanwhile are kept and replayed in order
  // on top of its answer: a delta can't be placed relative to the answer (the engine may flush
  // a later commit before the reply, and the reply's promise settles after the deltas read in
  // the same chunk), but each carries a category's full counts, so replaying them all ends on
  // the newest value for every category they touch.
  _loading: { deltas: ThreadCountsDelta[] } | null = null;

  constructor(engine: EngineEventsType = EngineEvents) {
    super();
    this._engine = engine;

    if (AppEnv.isMainWindow()) {
      // For now, unread counts are only retrieved in the main window.
      engine.on('connected', this._onEngineConnected);
      engine.on('gone', this._onEngineGone);
      engine.on('thread-counts', this._onThreadCountsDelta);
      if (engine.features) this._onEngineConnected();
    }
  }

  _onEngineConnected = () => {
    this._connected = true;
    const loading: { deltas: ThreadCountsDelta[] } = { deltas: [] };
    this._loading = loading;
    this._engine.request<CountsGetResult>(Method.CountsGet, {}).then(
      (result) => {
        if (this._loading !== loading) return;
        this._loading = null;
        let next: Counts = {};
        const counts = (result && result.counts) || {};
        for (const id of Object.keys(counts)) {
          next[id] = { unread: counts[id].unread, total: counts[id].total };
        }
        for (const delta of loading.deltas) {
          next = applyDelta(next, delta);
        }
        this._setCounts(next);
      },
      (err) => {
        if (this._loading !== loading) return;
        // The deltas still apply on top of the counts there were.
        console.warn(`counts.get failed: ${err}`);
        this._loading = null;
        let next = this._counts;
        for (const delta of loading.deltas) next = applyDelta(next, delta);
        this._setCounts(next);
      }
    );
  };

  // The counts stay as they were; the next hello reads them again.
  _onEngineGone = () => {
    this._connected = false;
    this._loading = null;
  };

  _onThreadCountsDelta = (delta: ThreadCountsDelta) => {
    if (!this._connected) return;
    if (this._loading) {
      this._loading.deltas.push(delta);
      return;
    }
    this._setCounts(applyDelta(this._counts, delta));
  };

  _setCounts(nextCounts: Counts) {
    if (_.isEqual(nextCounts, this._counts)) {
      return;
    }
    this._counts = nextCounts;
    this.trigger();
  }

  unreadCountForCategoryId(catId: string) {
    if (this._counts[catId] === undefined) {
      return null;
    }
    return this._counts[catId]['unread'];
  }

  totalCountForCategoryId(catId: string) {
    if (this._counts[catId] === undefined) {
      return null;
    }
    return this._counts[catId]['total'];
  }
}

function applyDelta(counts: Counts, { type, rows }: ThreadCountsDelta): Counts {
  const next: Counts = { ...counts };
  for (const row of rows || []) {
    if (!row || !row.categoryId) continue;
    if (type === 'unpersist') {
      delete next[row.categoryId];
    } else if (type === 'persist') {
      next[row.categoryId] = { unread: row.unread || 0, total: row.total || 0 };
    }
  }
  return next;
}

export default new ThreadCountsStore();
