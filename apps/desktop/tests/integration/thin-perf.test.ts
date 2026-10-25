// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

// The read measurement of docs/thin-clients-plan.md 4.7: the desktop's own read paths, run
// end to end in this window (query building, SQLite or the socket, JSON, model inflation, the
// thread list's message join) with `core.engine.reads` on each value, against a large
// synthetic profile and a real `serve` engine on it. It is skipped unless
// apps/engine/test/tools/thin_perf.py runs it, which builds the profile, starts the engine
// and prints the table docs/thin-clients/perf.md holds:
//
//   TERN_PERF_PROFILE=<dir with tern.db and profile.json> TERN_PERF_SOCKET=<serve socket>
//   TERN_PERF_OUT=<results.json> npm test -- --spec-file-pattern=thin-perf
import fs from 'fs';
import path from 'path';
import Sqlite3 from 'better-sqlite3';
import '../../src/data/query/observables';
import DatabaseStore from '../../src/data/database/store';
import EngineReads from '../../src/data/database/engine-reads';
import EngineEvents from '../../src/engine/client/events';
import { EngineConnection } from '../../src/engine/client/connection';
import { applyDeltas, feedEngineEvents } from '../../src/engine/client/bridge';
import { expandEvents } from '../../src/engine/client/pim';
import { Method } from '../../src/engine/protocol';
import { AccountStore } from '../../src/data/stores/account';
import { Account } from '../../src/data/models/account';
import { Thread } from '../../src/data/models/thread';
import { Message } from '../../src/data/models/message';
import { Folder } from '../../src/data/models/folder';
import { Label } from '../../src/data/models/label';
import { Category } from '../../src/data/models/category';
import { Contact } from '../../src/data/models/contact';
import { Calendar } from '../../src/data/models/calendar';
import { Event } from '../../src/data/models/event';
import { Matcher } from '../../src/data/attributes';
import { MailboxPerspective } from '../../src/data/mailbox-perspective';
import { MutableQuerySubscription } from '../../src/data/query/mutable-subscription';
import { QueryResultSet } from '../../src/data/query/result-set';
import ThreadListDataSource from '../../src/features/thread-list/data-source';
import SearchQuerySubscription from '../../src/features/thread-search/query-subscription';
import { ChangeStarredTask } from '../../src/data/tasks/change-starred';
import TimeOverride from '../support/runner/time-override';

const PROFILE = process.env.TERN_PERF_PROFILE;
const SOCKET = process.env.TERN_PERF_SOCKET;
const OUT = process.env.TERN_PERF_OUT;
const ITERATIONS = Number(process.env.TERN_PERF_ITERATIONS || 20);
// Rows on screen at 1000px and the thread list's row height, plus the list's 2-row margin.
const VISIBLE = 32;
const SCROLL_STEP = 25;
const SCROLL_ROWS = Number(process.env.TERN_PERF_SCROLL_ROWS || 1500);

type Mode = 'sqlite' | 'engine';
const MODES: Mode[] = ['sqlite', 'engine'];

interface ProfileAccount {
  id: string;
  emailAddress: string;
  gmail: boolean;
  categories: { [role: string]: string };
  longThread: string;
  calendars: string[];
}

interface Profile {
  now: number;
  accounts: ProfileAccount[];
  totals: { [k: string]: number };
}

type Samples = { [mode: string]: number[] };
const results: { [scenario: string]: { samples: Samples; note?: string; rows?: number } } = {};

function record(scenario: string, mode: Mode | 'both', ms: number, rows?: number) {
  const entry = (results[scenario] = results[scenario] || { samples: {} });
  (entry.samples[mode] = entry.samples[mode] || []).push(ms);
  if (rows !== undefined) entry.rows = rows;
}

function note(scenario: string, text: string) {
  results[scenario] = results[scenario] || { samples: {} };
  results[scenario].note = text;
}

const now = () => performance.now();
const immediate = () => new Promise((resolve) => setImmediate(resolve));

function setMode(mode: Mode) {
  EngineReads._flag = () => mode;
}

// Each iteration runs the modes in alternating order, so neither always reads a warmer cache.
function modesFor(i: number): Mode[] {
  return i % 2 === 0 ? MODES : [...MODES].reverse();
}

// Takes a promise or a ModelQuery, which runs when awaited.
async function timed(fn: () => any): Promise<[number, any]> {
  const start = now();
  const value = await fn();
  return [now() - start, value];
}

// Opens a thread list the way the list does and resolves with the time to the first result set
// covering `range`, threads joined with their messages.
function openList(
  subscription: MutableQuerySubscription<Thread>,
  range = { start: 0, end: VISIBLE }
) {
  const ds = new ThreadListDataSource(subscription);
  let last: QueryResultSet<Thread> | null = null;
  let waiter: { test: (rs: QueryResultSet<Thread>) => boolean; resolve: () => void } | null = null;
  ds.listen(({ next }: { next: QueryResultSet<Thread> }) => {
    last = next;
    if (waiter && waiter.test(next)) {
      const w = waiter;
      waiter = null;
      w.resolve();
    }
  }, null);
  const covers = (want: { offset: number; limit: number }) => (rs: QueryResultSet<Thread>) => {
    // The set is clipped to the rows there are.
    const r = rs.range();
    return r.offset === want.offset && r.limit <= want.limit;
  };
  const waitFor = (test: (rs: QueryResultSet<Thread>) => boolean, what: string) =>
    new Promise<void>((resolve, reject) => {
      if (last && test(last)) {
        resolve();
        return;
      }
      const timer = setTimeout(
        () => {
          const got = last ? JSON.stringify(last.range()) : 'nothing';
          reject(new Error(`timed out: ${what}, the list has ${got}`));
        },
        Number(process.env.TERN_PERF_WAIT_MS || 60000)
      );
      waiter = {
        test,
        resolve: () => {
          clearTimeout(timer);
          resolve();
        },
      };
    });
  const queryRange = () => {
    const r = subscription.query().range();
    return { offset: r.offset, limit: r.limit };
  };
  const setRange = async (next: { start: number; end: number }) => {
    const before = queryRange();
    const start = now();
    ds.setRetainedRange(next);
    const want = queryRange();
    if (want.offset === before.offset && want.limit === before.limit && last) return null;
    await waitFor(covers(want), `range ${JSON.stringify(want)}`);
    return now() - start;
  };
  return {
    ds,
    setRange,
    first: () => setRange(range),
    current: () => last,
    waitFor,
    close: () => ds.cleanup(),
  };
}

let profile: Profile;
let connection: EngineConnection;

async function connect() {
  if (connection && EngineEvents.features) return;
  connection = new EngineConnection({ clientName: 'tern-desktop/thin-perf', mainWindow: false });
  feedEngineEvents(connection, EngineEvents);
  connection.on('deltas', (msgs: string[]) => applyDeltas(msgs, { mainWindow: false }));
  const accounts = {};
  for (const a of profile.accounts) accounts[a.id] = { state: 'added', held: false };
  connection.setStatus({ generation: 1, socketPath: SOCKET, accounts } as any);
  for (let i = 0; i < 2000 && !EngineEvents.features; i++) await immediate();
  if (!EngineEvents.features) throw new Error(`no hello from the engine at ${SOCKET}`);
}

function openDatabase(dbPath: string) {
  const db = new Sqlite3(dbPath, { readonly: true, timeout: 10000 });
  // As DatabaseStore's openDatabase sets them.
  db.pragma(`journal_mode = WAL`);
  db.pragma(`main.page_size = 8192`);
  db.pragma(`main.cache_size = 20000`);
  db.pragma(`main.synchronous = NORMAL`);
  return db;
}

async function categories(): Promise<{ [key: string]: Category }> {
  setMode('sqlite');
  const folders = await DatabaseStore.findAll<Folder>(Folder);
  const labels = await DatabaseStore.findAll<Label>(Label);
  const byId: { [id: string]: Category } = {};
  for (const c of [...folders, ...labels]) byId[c.id] = c;
  const out: { [key: string]: Category } = {};
  for (const a of profile.accounts) {
    for (const [role, id] of Object.entries(a.categories)) out[`${a.id}:${role}`] = byId[id];
  }
  return out;
}

async function threadLists(cats: { [key: string]: Category }) {
  const [gmail, work, home] = profile.accounts.map((a) => a.id);
  const lists: { [name: string]: () => MutableQuerySubscription<Thread> } = {
    'unified inbox': () =>
      MailboxPerspective.forCategories([
        cats[`${gmail}:inbox`],
        cats[`${work}:inbox`],
        cats[`${home}:inbox`],
      ]).threads() as MutableQuerySubscription<Thread>,
    'gmail inbox (label)': () =>
      MailboxPerspective.forCategories([cats[`${gmail}:inbox`]]).threads() as any,
    'imap inbox (folder)': () =>
      MailboxPerspective.forCategories([cats[`${work}:inbox`]]).threads() as any,
    'unified sent': () =>
      MailboxPerspective.forCategories([
        cats[`${gmail}:sent`],
        cats[`${work}:sent`],
        cats[`${home}:sent`],
      ]).threads() as any,
    'gmail all mail': () =>
      MailboxPerspective.forCategories([cats[`${gmail}:all`]]).threads() as any,
    'gmail label': () =>
      MailboxPerspective.forCategories([cats[`${gmail}:label`]]).threads() as any,
    starred: () => MailboxPerspective.forStarred([gmail, work, home]).threads() as any,
    unread: () =>
      MailboxPerspective.forUnread([
        cats[`${gmail}:inbox`],
        cats[`${work}:inbox`],
        cats[`${home}:inbox`],
      ]).threads() as any,
  };
  return lists;
}

async function measureFirstPages(lists: {
  [name: string]: () => MutableQuerySubscription<Thread>;
}) {
  for (const [name, make] of Object.entries(lists)) {
    for (let i = 0; i < ITERATIONS + 2; i++) {
      for (const mode of modesFor(i)) {
        setMode(mode);
        const list = openList(make());
        const ms = await list.first();
        const rows = list.current().ids().length;
        list.close();
        // The first two are warm-up.
        if (i >= 2) record(`first page: ${name}`, mode, ms, rows);
        await immediate();
      }
    }
  }
}

async function measureScroll(make: () => MutableQuerySubscription<Thread>, name: string) {
  for (let pass = 0; pass < 3; pass++) {
    for (const mode of modesFor(pass)) {
      setMode(mode);
      const list = openList(make());
      await list.first();
      const total = Number(
        await DatabaseStore.count(Thread, list.ds._resultSet.query().matchers())
      );
      const deepest = Math.min(SCROLL_ROWS, total - VISIBLE);
      for (let top = SCROLL_STEP; top <= deepest; top += SCROLL_STEP) {
        const ms = await list.setRange({ start: top, end: top + VISIBLE });
        if (ms !== null) record(`scroll step: ${name}`, mode, ms);
      }
      list.close();
      await immediate();
    }
  }
}

async function measureCounts() {
  const dbPath = path.join(PROFILE, 'tern.db');
  const db = openDatabase(dbPath);
  for (let i = 0; i < ITERATIONS + 2; i++) {
    const [engineMs, result] = await timed(() =>
      EngineEvents.request<{ counts: object }>(Method.CountsGet, {})
    );
    const [sqlMs, rows] = await timed(async () => db.prepare('SELECT * FROM ThreadCounts').all());
    if (i >= 2) {
      record(
        'unread counts: counts.get (both values)',
        'engine',
        engineMs,
        Object.keys(result.counts).length
      );
      record('unread counts: counts.get (both values)', 'sqlite', sqlMs, rows.length);
    }
  }
  note(
    'unread counts: counts.get (both values)',
    'ThreadCountsStore reads counts.get whatever the flag says (stage 0); the sqlite column is the removed ThreadCounts SELECT, for scale'
  );
  db.close();
  const inbox = profile.accounts.map((a) => a.categories.inbox);
  for (let i = 0; i < ITERATIONS + 2; i++) {
    for (const mode of modesFor(i)) {
      setMode(mode);
      const [ms] = await timed(() =>
        DatabaseStore.count<Thread>(Thread, [
          Thread.attributes.categories.containsAny(inbox),
          Thread.attributes.unread.equal(true),
        ])
      );
      if (i >= 2) record('unread count query (unified inbox)', mode, ms);
    }
  }
}

async function measureSearch(accountIds: string[]) {
  const searches = [
    'budget',
    'from:smith',
    'is:unread in:inbox',
    '"design review"',
    'zebra quokka',
    'subject:launch -in:trash',
  ];
  for (const text of searches) {
    for (let i = 0; i < Math.max(5, ITERATIONS / 2) + 1; i++) {
      for (const mode of modesFor(i)) {
        setMode(mode);
        const sub = new SearchQuerySubscription(text, accountIds);
        const list = openList(sub as any);
        list.ds.setRetainedRange({ start: 0, end: VISIBLE });
        const start = now();
        await list.waitFor((rs) => rs.ids().length >= 0, `search ${text}`);
        const ms = now() - start;
        const rows = list.current().ids().length;
        list.close();
        sub.onLastCallbackRemoved();
        if (i >= 1) record(`search (grammar): ${text}`, mode, ms, rows);
        await immediate();
      }
    }
  }
  note(
    'search (grammar): budget',
    'the grammar always runs in the engine; the flag changes the messages-of-threads join under the results (up to 1000 threads)'
  );
  // The plain FTS search a few call sites still send (subject suggestions, the fallback).
  for (let i = 0; i < ITERATIONS + 2; i++) {
    for (const mode of modesFor(i)) {
      setMode(mode);
      const [ms, rows] = await timed(() =>
        DatabaseStore.findAll<Thread>(Thread)
          .search('glacier')
          .order(Thread.attributes.lastMessageReceivedTimestamp.descending())
          .limit(100)
      );
      if (i >= 2) record('search (plain FTS, 100 threads)', mode, ms, rows.length);
    }
  }
}

async function openThread(threadId: string, allBodies: boolean) {
  const messages = await DatabaseStore.findAll<Message>(Message).where({ threadId });
  const shown = messages.filter((m) => !m.isHidden());
  // MessageStore expands the unread ones and the last one by default.
  const expanded = allBodies
    ? shown
    : shown.filter((m, idx) => m.unread || idx === shown.length - 1);
  await DatabaseStore.findAll<Message>(Message, { id: expanded.map((m) => m.id) }).include(
    Message.attributes.body
  );
  return shown.length;
}

async function measureMessageOpen() {
  for (const a of profile.accounts) {
    for (const [label, all] of [
      ['default expansion', false],
      ['all bodies', true],
    ] as const) {
      for (let i = 0; i < ITERATIONS + 2; i++) {
        for (const mode of modesFor(i)) {
          setMode(mode);
          const [ms, rows] = await timed(() => openThread(a.longThread, all));
          if (i >= 2) record(`open 50-message thread, ${label} (${a.id})`, mode, ms, rows);
        }
      }
    }
  }
}

async function measureContacts() {
  const shapes: { [name: string]: () => any } = {
    'contact autocomplete "ann"': () =>
      DatabaseStore.findAll<Contact>(Contact)
        .search('ann')
        .limit(5 * 3)
        .where(Contact.attributes.refs.greaterThan(0))
        .where(Contact.attributes.hidden.equal(false))
        .order(Contact.attributes.refs.descending()),
    'top contacts': () =>
      DatabaseStore.findAll<Contact>(Contact)
        .limit(5 * 3)
        .where(Contact.attributes.refs.greaterThan(0))
        .where(Contact.attributes.hidden.equal(false))
        .order(Contact.attributes.refs.descending()),
    'contacts list (all with refs)': () =>
      DatabaseStore.findAll<Contact>(Contact)
        .where(Contact.attributes.refs.greaterThan(0))
        .where(Contact.attributes.hidden.equal(false)),
    'contacts list (address book)': () =>
      DatabaseStore.findAll<Contact>(Contact).where(Contact.attributes.source.not('mail')),
  };
  for (const [name, run] of Object.entries(shapes)) {
    for (let i = 0; i < ITERATIONS + 2; i++) {
      for (const mode of modesFor(i)) {
        setMode(mode);
        const [ms, rows] = await timed(run);
        if (i >= 2) record(name, mode, ms, rows.length);
      }
    }
  }
}

async function measureCalendar() {
  const calendarIds = profile.accounts.flatMap((a) => a.calendars);
  const day = 86400;
  for (let i = 0; i < ITERATIONS + 2; i++) {
    for (const mode of modesFor(i)) {
      setMode(mode);
      const [ms, rows] = await timed(() => DatabaseStore.findAll<Calendar>(Calendar));
      if (i >= 2) record('calendars', mode, ms, rows.length);
    }
  }
  for (const [label, span] of [
    ['week', 7 * day],
    ['month', 35 * day],
  ] as const) {
    const start = profile.now - (profile.now % day);
    for (let i = 0; i < ITERATIONS + 2; i++) {
      const [ms, result] = await timed(() =>
        expandEvents({ calendarIds, start, end: start + span })
      );
      if (i >= 2) {
        record(
          `calendar ${label}: events.expand (both values)`,
          'engine',
          ms,
          (result.occurrences || []).length
        );
      }
    }
    note(
      `calendar ${label}: events.expand (both values)`,
      'the calendar reads events.expand whatever the flag says (stage 0)'
    );
    // The Event range query older call sites build, for the flag's own cost.
    const startUnix = start;
    const endUnix = start + span;
    for (let i = 0; i < ITERATIONS + 2; i++) {
      for (const mode of modesFor(i)) {
        setMode(mode);
        const end = Event.attributes.recurrenceEnd;
        const begin = Event.attributes.recurrenceStart;
        const [ms, rows] = await timed(() =>
          DatabaseStore.findAll<Event>(Event).where(
            new Matcher.Or([
              new Matcher.And([begin.lte(endUnix), end.gte(startUnix)]),
              new Matcher.And([begin.lte(endUnix), begin.gte(startUnix)]),
              new Matcher.And([end.gte(startUnix), end.lte(endUnix)]),
              new Matcher.And([end.gte(endUnix), begin.lte(startUnix)]),
            ])
          )
        );
        if (i >= 2) record(`calendar ${label}: Event range query`, mode, ms, rows.length);
      }
    }
  }
}

// Where a first page's time goes: its two reads (the 200 threads, then their messages for the
// list's join) split into the read itself and model inflation, on each transport, plus the
// engine's answer size and the same read as idsOnly (the SQL without the rows).
async function measureBreakdown(make: () => MutableQuerySubscription<Thread>, name: string) {
  setMode('sqlite');
  const probe = make();
  const threadsQuery = probe.query().clone().page(0, VISIBLE);
  const threads = (await DatabaseStore.run(threadsQuery.clone(), { format: false })) as Thread[];
  const ids = threads.map((t) => t.id);
  const reads: { [label: string]: () => any } = {
    threads: () => threadsQuery.clone(),
    messages: () => DatabaseStore.findAll<Message>(Message, { threadId: ids }),
  };
  for (const [label, build] of Object.entries(reads)) {
    const key = `breakdown ${name}: ${label}`;
    for (let i = 0; i < ITERATIONS + 2; i++) {
      const sqlQuery = build();
      const [readMs, rows] = await timed(() => DatabaseStore._query(sqlQuery.sql(), []));
      const [inflateMs] = await timed(async () => sqlQuery.inflateResult(rows));
      const json = build().toJSON();
      const [rpcMs, result] = await timed(() => EngineEvents.request(Method.Query, json));
      const engineQuery = build();
      const [engineInflateMs] = await timed(async () => engineQuery.inflateEngineResult(result));
      const [idsMs] = await timed(() =>
        EngineEvents.request(Method.Query, { ...json, idsOnly: true })
      );
      if (i < 2) continue;
      record(`${key}, read`, 'sqlite', readMs, rows.length);
      record(`${key}, inflate`, 'sqlite', inflateMs);
      record(`${key}, read`, 'engine', rpcMs, result.rows.length);
      record(`${key}, inflate`, 'engine', engineInflateMs);
      record(`${key}, idsOnly read`, 'engine', idsMs);
      if (i === 2) {
        const bytes = JSON.stringify(result).length;
        const sqlBytes = rows.reduce((n, r) => n + String(r.data || '').length, 0);
        note(
          `${key}, read`,
          `answer ${Math.round(bytes / 1024)} KB as JSON; the SQLite rows' data ${Math.round(sqlBytes / 1024)} KB`
        );
      }
    }
  }
}

// Star and unstar the top inbox thread: the time from queueing the task to the list showing it.
// The engine runs the local phase; its remote phase can't reach the (absent) server.
async function measureDeltaToRender(make: () => MutableQuerySubscription<Thread>) {
  for (let i = 0; i < ITERATIONS + 2; i++) {
    for (const mode of modesFor(i)) {
      setMode(mode);
      const list = openList(make());
      await list.first();
      const thread = list.current().modelAtOffset(0) as Thread;
      for (const starred of [!thread.starred, thread.starred]) {
        const task = new ChangeStarredTask({ threads: [thread], starred, source: 'perf' });
        const start = now();
        if (!connection.send(thread.accountId, { type: 'queue-task', task: task.toJSON() })) {
          throw new Error(`the engine doesn't run ${thread.accountId}`);
        }
        await list.waitFor((rs) => {
          const t = rs.modelWithId(thread.id) as Thread;
          return !!t && t.starred === starred;
        }, 'the starred delta');
        if (i >= 2) record('delta to render: star in inbox', mode, now() - start);
      }
      list.close();
      await immediate();
    }
  }
  note(
    'delta to render: star in inbox',
    'task.queue, the local phase, the delta, the list patching its row (no React render)'
  );
}

(PROFILE && SOCKET ? describe : xdescribe)('thin-clients read measurement', function () {
  let defaultTimeout: number;

  beforeEach(function () {
    TimeOverride.disableSpies();
    spyOn(console, 'log');
    (DatabaseStore._query as any).andCallThrough();
    defaultTimeout = jasmine.getEnv().defaultTimeoutInterval;
    jasmine.getEnv().defaultTimeoutInterval = 60 * 60 * 1000;
    profile = JSON.parse(fs.readFileSync(path.join(PROFILE, 'profile.json'), 'utf8'));
    const dbPath = path.join(PROFILE, 'tern.db');
    DatabaseStore._databasePath = dbPath;
    DatabaseStore._db = openDatabase(dbPath);
    DatabaseStore._preparedStatementCache.clear();
    DatabaseStore._open = true;
    (AccountStore as any)._accounts = profile.accounts.map(
      (a) =>
        new Account({
          id: a.id,
          emailAddress: a.emailAddress,
          name: 'Perf User',
          provider: a.gmail ? 'gmail' : 'imap',
        } as any)
    );
  });

  afterEach(function () {
    jasmine.getEnv().defaultTimeoutInterval = defaultTimeout;
    TimeOverride.enableSpies();
  });

  it('measures the reads with core.engine.reads on each value', async function () {
    await connect();
    const cats = await categories();
    const lists = await threadLists(cats);
    const started = now();
    await measureFirstPages(lists);
    await measureScroll(lists['unified inbox'], 'unified inbox');
    await measureScroll(lists['gmail all mail'], 'gmail all mail');
    await measureCounts();
    await measureSearch(profile.accounts.map((a) => a.id));
    await measureMessageOpen();
    await measureContacts();
    await measureCalendar();
    await measureBreakdown(lists['unified inbox'], 'unified inbox');
    await measureBreakdown(lists['unified sent'], 'unified sent');
    await measureDeltaToRender(lists['unified inbox']);
    const out = {
      generatedBy: 'apps/desktop/tests/integration/thin-perf.test.ts',
      profile: profile.totals,
      iterations: ITERATIONS,
      agentUnavailable: DatabaseStore._agentUnavailable,
      seconds: Math.round((now() - started) / 1000),
      results,
    };
    fs.writeFileSync(OUT || path.join(PROFILE, 'results.json'), JSON.stringify(out, null, 2));
    expect(Object.keys(results).length).toBeGreaterThan(10);
  });
});
