import _ from 'underscore';
import MailspringStore from 'mailspring-store';
import DatabaseStore from './database-store';
import { Thread } from '../models/thread';
import EngineEvents, {
  EngineEvents as EngineEventsType,
  ThreadCountsDelta,
} from '../../engine-events';
import { CountsGetResult, Feature, Method } from '../../engine-protocol';

interface ThreadCountRow {
  categoryId: string;
  unread: number;
  total: number;
}

type Counts = { [categoryId: string]: { unread: number; total: number } };

/*
Unread and total thread counts per folder and label. An engine with the `counts` feature
answers `counts.get` once per connection and sends every change after it as a ThreadCounts
delta; without one (per-account engines, the C++ engine, an older Rust engine) the counts are
read from the ThreadCounts table whenever threads change.
*/
export class ThreadCountsStore extends MailspringStore {
  _counts: Counts = {};
  _engine: EngineEventsType;
  // The counts come from the engine rather than the table.
  _fromEngine = false;
  // Set while counts.get is out. Deltas that arrive meanwhile are kept and replayed in order
  // on top of its answer: a delta can't be placed relative to the answer (the engine may flush
  // a later commit before the reply, and the reply's promise settles after the deltas read in
  // the same chunk), but each carries a category's full counts, so replaying them all ends on
  // the newest value for every category they touch.
  _loading: { deltas: ThreadCountsDelta[] } = null;

  constructor(engine: EngineEventsType = EngineEvents) {
    super();
    this._engine = engine;

    if (AppEnv.isMainWindow()) {
      // For now, unread counts are only retrieved in the main window.
      const onCountsChangedDebounced = _.throttle(this._onCountsChanged, 1000);
      DatabaseStore.listen((change) => {
        if (change.objectClass === Thread.name && !this._fromEngine) {
          onCountsChangedDebounced();
        }
      });
      engine.on('connected', this._onEngineConnected);
      engine.on('gone', this._onEngineGone);
      engine.on('thread-counts', this._onThreadCountsDelta);
      if (engine.has(Feature.Counts)) {
        this._onEngineConnected();
      } else {
        onCountsChangedDebounced();
      }
    }
  }

  _onCountsChanged = () => {
    DatabaseStore._query('SELECT * FROM `ThreadCounts`').then((results: ThreadCountRow[]) => {
      if (this._fromEngine) return;
      const nextCounts = {};
      for (const { categoryId, unread, total } of results) {
        nextCounts[categoryId] = { unread, total };
      }
      this._setCounts(nextCounts);
    });
  };

  _onEngineConnected = () => {
    if (!this._engine.has(Feature.Counts)) {
      this._fromEngine = false;
      this._loading = null;
      this._onCountsChanged();
      return;
    }
    this._fromEngine = true;
    const loading = { deltas: [] };
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
        this._loading = null;
        console.warn(`counts.get failed, reading the ThreadCounts table instead: ${err}`);
        this._fromEngine = false;
        this._onCountsChanged();
      }
    );
  };

  // The counts stay as they were; the next hello reads them again.
  _onEngineGone = () => {
    this._fromEngine = false;
    this._loading = null;
  };

  _onThreadCountsDelta = (delta: ThreadCountsDelta) => {
    if (!this._fromEngine) return;
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
