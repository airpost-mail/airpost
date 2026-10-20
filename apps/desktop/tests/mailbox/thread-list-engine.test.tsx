// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import React from 'react';
import ReactDOM from 'react-dom';
import { ipcRenderer } from 'electron';
import { Folder, Message, Thread, MailboxPerspective } from 'mailspring-exports';
import ThreadListDataSource from '../../src/features/thread-list/data-source';
import EngineEvents from '../../src/engine/client/events';
import EngineReads, { READ_WAIT_MS } from '../../src/data/database/engine-reads';
import { SupervisorChannel } from '../../src/engine/channels';
import {
  EngineUnavailableState,
  SHOW_WAITING_AFTER_MS,
} from '../../src/components/engine-unavailable-state';
import { MockEngine, settle } from '../support/mock-engine';

const THREADS = 1500;
const ROWS = 50;

const inbox = new Folder({ id: 'inbox', accountId: TEST_ACCOUNT_ID, role: 'inbox', path: 'INBOX' });

async function until(what: string, condition: () => boolean) {
  for (let i = 0; i < 400; i++) {
    if (condition()) return;
    await settle(5);
    advanceClock(1);
  }
  throw new Error(`Timed out waiting for ${what}`);
}

describe('the thread list on the engine', function () {
  let engine: MockEngine = null;
  let dataSource: ThreadListDataSource = null;

  beforeEach(async function () {
    engine = new MockEngine().install();
    for (let i = 0; i < THREADS; i++) {
      const ts = 2000000000 - i * 60;
      engine.add(
        new Thread({
          id: `t${i}`,
          accountId: TEST_ACCOUNT_ID,
          subject: `Subject ${i}`,
          folders: [inbox],
          inAllMail: true,
          unread: i % 3 === 0,
          starred: false,
          lastMessageReceivedTimestamp: new Date(ts * 1000),
        }),
        new Message({
          id: `m${i}`,
          accountId: TEST_ACCOUNT_ID,
          threadId: `t${i}`,
          date: new Date(ts * 1000),
          subject: `Subject ${i}`,
          snippet: 'Hello',
        })
      );
    }
    await settle();
    dataSource = new ThreadListDataSource(MailboxPerspective.forCategory(inbox).threads());
    dataSource.setRetainedRange({ start: 0, end: ROWS });
    await until('the first page', () => !!dataSource.get(ROWS - 1));
    engine.requests = [];
  });

  afterEach(function () {
    dataSource.cleanup();
    engine.uninstall();
    EngineEvents.supervisorStatus(null);
    advanceClock(10);
  });

  it('scrolls through the inbox with a bounded number of reads per page', async function () {
    let pages = 0;
    const started = performance.now();
    for (let top = 25; top + ROWS <= THREADS; top += 25) {
      const before = engine.requests.length;
      dataSource.setRetainedRange({ start: top, end: top + ROWS });
      await until(`rows ${top}-${top + ROWS}`, () => !!dataSource.get(top + ROWS - 1));
      if (engine.requests.length > before) pages += 1;
      // The thread page (ids, then the models missing), and the messages of the new threads.
      expect(engine.requests.length - before).toBeLessThan(4);
    }
    const msec = performance.now() - started;
    const perPage = msec / Math.max(pages, 1);
    console.log(
      `Thread list scroll: ${pages} pages, ${msec.toFixed(0)} ms, ${perPage.toFixed(1)} ms per page`
    );
    expect(pages).toBeGreaterThan(10);
    // The mock answers in-process, so this is the renderer's own cost per page: inflating,
    // diffing and the message join. Generous, to catch quadratic work rather than noise.
    expect(perPage).toBeLessThan(50);
  });

  it('waits while the engine is gone, re-runs after the respawn, and shows its state after give-up', async function () {
    engine.kill();
    dataSource.setRetainedRange({ start: 400, end: 400 + ROWS });
    await settle(40);
    // Nothing was sent, and the list keeps its rows.
    expect(engine.requests).toEqual([]);
    expect(dataSource.get(0).id).toBe('t0');
    expect(dataSource.get(400)).toBeFalsy();

    advanceClock(READ_WAIT_MS / 2);
    engine.start();
    await until('rows after the respawn', () => !!dataSource.get(400 + ROWS - 1));
    expect(dataSource.get(400).id).toBe('t400');
    expect(engine.queries('Thread').length).toBeGreaterThan(0);

    // The engine dies for good: reads give up, the list keeps its rows, the state shows.
    spyOn(console, 'warn');
    engine.kill();
    EngineEvents.supervisorStatus({
      generation: null,
      socketPath: null,
      gaveUp: true,
      accounts: {},
    });
    expect(EngineReads.unavailable()).toBe('gave-up');
    dataSource.setRetainedRange({ start: 800, end: 800 + ROWS });
    await settle(20);
    advanceClock(READ_WAIT_MS);
    await settle(20);
    expect(dataSource.get(400).id).toBe('t400');

    const node = document.createElement('div');
    ReactDOM.render(
      <EngineUnavailableState>
        <span>Empty</span>
      </EngineUnavailableState>,
      node
    );
    await settle();
    expect(node.textContent).toContain('The sync engine stopped');
    spyOn(ipcRenderer, 'invoke').andReturn(Promise.resolve());
    (node.querySelector('button') as HTMLButtonElement).click();
    expect(ipcRenderer.invoke).toHaveBeenCalledWith(SupervisorChannel.RetryEngine);

    // The supervisor starts it again: the state goes, the list reads the rows it asked for.
    EngineEvents.supervisorStatus({ generation: 2, socketPath: '/x', accounts: {} });
    engine.start();
    await until('rows after Try Again', () => !!dataSource.get(800 + ROWS - 1));
    await settle();
    expect(node.textContent).toBe('Empty');
    ReactDOM.unmountComponentAtNode(node);
  });

  it('shows nothing during a short reconnect', async function () {
    const node = document.createElement('div');
    ReactDOM.render(
      <EngineUnavailableState>
        <span>Empty</span>
      </EngineUnavailableState>,
      node
    );
    await settle();
    engine.kill();
    await settle();
    expect(node.textContent).toBe('Empty');
    advanceClock(SHOW_WAITING_AFTER_MS);
    await settle();
    expect(node.textContent).toContain('Waiting for the sync engine');
    engine.start();
    await settle();
    expect(node.textContent).toBe('Empty');
    ReactDOM.unmountComponentAtNode(node);
  });
});
