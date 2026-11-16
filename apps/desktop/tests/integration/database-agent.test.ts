// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

// DatabaseStore's background queries through the real agent: main forks agent.js as a utility
// process (main/ipc/database-agent.ts), so no part of it needs ELECTRON_RUN_AS_NODE.
import fs from 'fs';
import os from 'os';
import path from 'path';
import Sqlite3 from 'better-sqlite3';
import DatabaseStore from '../../src/data/database/store';

describe('the background query agent', function () {
  let dir: string;
  let savedPath: string;

  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'airpost-agent-spec-'));
    const dbPath = path.join(dir, 'agent.db');
    const db = new Sqlite3(dbPath);
    db.exec(`CREATE TABLE T (id TEXT, n INTEGER); INSERT INTO T VALUES ('a', 1), ('b', 2);`);
    db.close();
    savedPath = DatabaseStore._databasePath;
    DatabaseStore._databasePath = dbPath;
  });

  afterEach(() => {
    DatabaseStore._databasePath = savedPath;
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('answers a query from a utility process', () => {
    let response: any = null;
    let localRuns = 0;
    spyOn(DatabaseStore, '_executeLocally').andCallFake(async () => {
      localRuns += 1;
      return [];
    });
    DatabaseStore._executeInBackground('SELECT id, n FROM T WHERE n > ? ORDER BY n', [0]).then(
      (r) => (response = r)
    );
    waitsFor(() => response !== null, 'the agent to answer', 10000);
    runs(() => {
      expect(localRuns).toBe(0);
      expect(response.results).toEqual([
        { id: 'a', n: 1 },
        { id: 'b', n: 2 },
      ]);
      expect(response.agentTime).not.toBeLessThan(0);
    });
  });

  it('runs a query the agent fails on in the window', () => {
    let response: any = null;
    spyOn(DatabaseStore, '_executeLocally').andReturn(Promise.resolve([{ local: true }]));
    DatabaseStore._executeInBackground('SELECT nope FROM Missing', []).then((r) => (response = r));
    waitsFor(() => response !== null, 'the fallback', 10000);
    runs(() => expect(response).toEqual({ results: [{ local: true }], agentTime: -1 }));
  });
});
