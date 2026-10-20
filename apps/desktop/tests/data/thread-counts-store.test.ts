// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import DatabaseStore from '../../src/data/database/store';
import { ThreadCountsStore } from '../../src/data/stores/thread-counts';
import { EngineEvents } from '../../src/engine/events';
import { Feature, Method } from '../../src/shared/engine/protocol';

const flush = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

describe('ThreadCountsStore', function () {
  beforeEach(function () {
    spyOn(AppEnv, 'isMainWindow').andReturn(true);
    this.tableRows = [{ categoryId: 'c1', unread: 1, total: 2 }];
    // master-before-each already spies on it.
    this.query = DatabaseStore._query as any;
    this.query.andCallFake(() => Promise.resolve(this.tableRows));
    this.engine = new EngineEvents();
    this.requests = [];
    this.answer = { counts: { c1: { unread: 3, total: 5 }, c2: { unread: 0, total: 4 } } };
    this.request = (method: string, params: unknown) => {
      this.requests.push({ method, params });
      return this.answer instanceof Error
        ? Promise.reject(this.answer)
        : Promise.resolve(this.answer);
    };
  });

  const counts = (store: ThreadCountsStore, ids: string[]) =>
    ids.map((id) => [store.unreadCountForCategoryId(id), store.totalCountForCategoryId(id)]);

  it('reads the ThreadCounts table without an engine that has counts', function () {
    waitsForPromise(async () => {
      const store = new ThreadCountsStore(this.engine);
      await flush();
      expect(DatabaseStore._query).toHaveBeenCalled();
      expect(counts(store, ['c1', 'c9'])).toEqual([
        [1, 2],
        [null, null],
      ]);

      // An engine without the feature leaves it on the table, deltas and all.
      this.engine.connected([Feature.MailNew], this.request);
      await flush();
      this.engine.threadCounts({
        type: 'persist',
        rows: [{ id: 'c1', categoryId: 'c1', unread: 9, total: 9 }],
      });
      expect(this.requests).toEqual([]);
      expect(counts(store, ['c1'])).toEqual([[1, 2]]);
    });
  });

  it('reads counts.get once connected and applies the ThreadCounts deltas after it', function () {
    waitsForPromise(async () => {
      const store = new ThreadCountsStore(this.engine);
      await flush();
      this.query.reset();
      let triggers = 0;
      store.listen(() => triggers++);

      this.engine.connected([Feature.Counts], this.request);
      // While counts.get is out: a delta the answer already includes, then (as when the reply
      // and later deltas arrive in one socket read) one newer than it.
      this.engine.threadCounts({
        type: 'persist',
        rows: [{ id: 'c1', categoryId: 'c1', unread: 3, total: 5 }],
      });
      this.engine.threadCounts({
        type: 'persist',
        rows: [{ id: 'c2', categoryId: 'c2', unread: 1, total: 4 }],
      });
      await flush();
      expect(this.requests).toEqual([{ method: Method.CountsGet, params: {} }]);
      expect(counts(store, ['c1', 'c2'])).toEqual([
        [3, 5],
        [1, 4],
      ]);
      expect(triggers).toEqual(1);
      this.engine.threadCounts({
        type: 'persist',
        rows: [{ id: 'c2', categoryId: 'c2', unread: 0, total: 4 }],
      });

      this.engine.threadCounts({
        type: 'persist',
        rows: [
          { id: 'c1', categoryId: 'c1', unread: 2, total: 5 },
          { id: 'c3', categoryId: 'c3', unread: 0, total: 0 },
        ],
      });
      expect(counts(store, ['c1', 'c3'])).toEqual([
        [2, 5],
        [0, 0],
      ]);
      this.engine.threadCounts({ type: 'unpersist', rows: [{ id: 'c2', categoryId: 'c2' }] });
      expect(counts(store, ['c2'])).toEqual([[null, null]]);
      // The same values again don't trigger.
      const before = triggers;
      this.engine.threadCounts({
        type: 'persist',
        rows: [{ id: 'c1', categoryId: 'c1', unread: 2, total: 5 }],
      });
      expect(triggers).toEqual(before);
      expect(DatabaseStore._query).not.toHaveBeenCalled();

      // Gone and back: counts.get again.
      this.engine.gone();
      this.answer = { counts: { c1: { unread: 0, total: 5 } } };
      this.engine.connected([Feature.Counts], this.request);
      await flush();
      expect(this.requests.length).toEqual(2);
      expect(counts(store, ['c1', 'c3'])).toEqual([
        [0, 5],
        [null, null],
      ]);
    });
  });

  it('ends on the newest counts when a delta during counts.get predates the answer', function () {
    waitsForPromise(async () => {
      const store = new ThreadCountsStore(this.engine);
      await flush();
      this.answer = { counts: { c1: { unread: 3, total: 5 } } };
      this.engine.connected([Feature.Counts], this.request);
      // Older than the answer, and followed by the delta of the commit the answer includes.
      this.engine.threadCounts({
        type: 'persist',
        rows: [{ id: 'c1', categoryId: 'c1', unread: 7, total: 7 }],
      });
      this.engine.threadCounts({
        type: 'persist',
        rows: [{ id: 'c1', categoryId: 'c1', unread: 3, total: 5 }],
      });
      this.engine.threadCounts({ type: 'unpersist', rows: [{ id: 'c4', categoryId: 'c4' }] });
      await flush();
      expect(counts(store, ['c1', 'c4'])).toEqual([
        [3, 5],
        [null, null],
      ]);
    });
  });

  it('falls back to the table when counts.get fails', function () {
    waitsForPromise(async () => {
      this.answer = new Error('unknown method counts.get');
      this.engine.connected([Feature.Counts], this.request);
      const store = new ThreadCountsStore(this.engine);
      await flush();
      expect(this.requests.length).toEqual(1);
      expect(counts(store, ['c1'])).toEqual([[1, 2]]);
      this.engine.threadCounts({
        type: 'persist',
        rows: [{ id: 'c1', categoryId: 'c1', unread: 9, total: 9 }],
      });
      expect(counts(store, ['c1'])).toEqual([[1, 2]]);
    });
  });
});
