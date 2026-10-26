# Reads: direct SQLite vs the engine's `query`

Measured 2026-10-25 for stage 2 of `docs/thin-clients-plan.md` (section 4.7). Result: the
section 4.7 targets hold on the profile below, so `core.engine.reads` now defaults to `engine`;
`"sqlite"` stays as the kill switch until stage 12. On a stress profile whose first pages carry
about 20 messages per thread, first pages miss the 30 ms target over the socket (and one of
them on SQLite too); see "Where the time goes".

## How it was measured

- **Profile.** Synthetic, no real mailbox: `cargo run --release -p tern-sync --example
  perf_profile -- <dir>` (`crates/tern-sync/examples/perf_profile.rs`). Every message goes
  through `MailProcessor` and `retrieved_message_body` as sync writes them (threading,
  placements, Gmail labels, ThreadCategory, ThreadCounts, ThreadSearch, mail contacts), and
  `tern_store::invariants::check` passes on the result. 100,014 messages in 32,552 threads
  over six years, 47,746 bodies (2-4 KB HTML), 34,796 contacts (30k from sent mail, 4,500
  CardDAV), 7 calendars with 5,600 events (a tenth weekly), 538 MB. Three accounts: a
  Gmail-style one (55k messages, labels, All Mail; inbox 2,688 threads), two IMAP ones (30k
  and 15k; inboxes 1,619 and 860). Thread lengths: 68% one message, 1% 31-120, mean 3.1;
  the 200 newest inbox threads average 2.3 messages. Each account has one 50-message thread.
- **Stress profile** (`--busy-top`): the same mail with each thread's messages held at the
  clock, so the long threads lead every list: the 200 newest inbox threads average about 20
  messages (3,700 on the unified inbox's first page).
- **Clients.** `apps/desktop/tests/integration/thin-perf.test.ts` in the desktop's spec window
  (Electron 44.5.1), on the real code paths: `MailboxPerspective.threads()` and
  `ThreadListDataSource` (the thread list's subscription with its messages join),
  `SearchQuerySubscription`, MessageStore's two reads, ContactStore's and the contacts
  app's queries, the calendar's `events.expand`, `counts.get`. Times are end to end in the
  window: query building, SQLite (better-sqlite3, the background agent for paging, as in
  the app) or the socket (JSON encode, the engine, JSON parse), model inflation, the
  messages join. Not React rendering, which is the same on both values. The two values
  alternate per iteration; two warm-up iterations are dropped.
- **Engine.** Release `tern-engine --mode serve` on a copy of the profile, the accounts
  added with an IMAP server that refuses connections, so it serves reads and runs task local
  phases and syncs nothing.
- **Machine.** Apple M4 Max, 64 GB, macOS 26.6.
- **Run it.** `CARGO_TARGET_DIR=... python3 apps/engine/test/tools/thin_perf.py [--busy-top]
  [--dir <dir>]` builds the profile under `/tmp/tern-shots/thin-perf` (or `<dir>`), starts the
  engine, runs the spec and prints the table. Since stage 6 the thread list scenarios also run
  with live queries off ("query + join" rows).

The first page is what the list loads for a 32-row screen: `page(0, 32, 50, 100)`, 200
threads, then the messages of those threads for the join. A scroll step moves the screen 25
rows, to 1,500 rows deep; only steps that read are counted (about one in two).

## Targets (4.7)

| Target | Profile | Stress profile |
|---|---|---|
| First page p95 < 30 ms | met: 11.6-23.9 ms (SQLite 4.9-9.9) | missed on 5 of 8 lists: 32.5-112.9 ms (SQLite 5.6-34.6, sent missing) |
| Scroll step p95 < 30 ms | met: 14.2, 15.8 ms (SQLite 17.4, 14.3) | met: 8.6, 15.4 ms |
| Open a 50-message thread < 50 ms | met: under 3 ms | met: under 2 ms |
| Delta to render < 50 ms | met: 7.3 ms (SQLite 7.3) | met: 8.5 ms |

Delta to render is starring the top inbox thread: `task.queue`, the engine's local phase, the
delta, the list patching its row. A star keeps the row in the list, so neither value reads
again; a change that moves rows (archive) re-reads the window, which costs what a scroll
step costs.

## Profile

| Shape | rows | SQLite p50 / p95 ms | engine p50 / p95 ms | n |
|---|---:|---:|---:|---:|
| first page: unified inbox | 200 | 6.2 / 7.4 | 13.6 / 15.4 | 20 |
| first page: gmail inbox (label) | 200 | 5.4 / 5.7 | 12.1 / 13.0 | 20 |
| first page: imap inbox (folder) | 200 | 5.1 / 5.4 | 11.7 / 12.7 | 20 |
| first page: unified sent | 200 | 8.8 / 9.1 | 18.5 / 23.9 | 20 |
| first page: gmail all mail | 200 | 9.4 / 9.9 | 19.5 / 23.1 | 20 |
| first page: gmail label | 200 | 4.4 / 4.9 | 10.4 / 11.6 | 20 |
| first page: starred | 200 | 6.5 / 6.9 | 15.9 / 17.3 | 20 |
| first page: unread | 200 | 6.6 / 7.0 | 14.4 / 20.5 | 20 |
| scroll step: unified inbox |  | 4.9 / 17.4 | 8.2 / 14.2 | 84 |
| scroll step: gmail all mail |  | 8.0 / 14.3 | 13.9 / 15.8 | 84 |
| unread counts: counts.get (both values) | 42 | 0.0 / 0.1 | 0.1 / 0.1 | 20 |
| unread count query (unified inbox) |  | 3.3 / 3.7 | 6.5 / 6.9 | 20 |
| search (grammar): budget | 792 | 29.3 / 45.1 | 48.3 / 64.5 | 10 |
| search (grammar): from:smith | 1000 | 49.5 / 105.3 | 88.1 / 191.7 | 10 |
| search (grammar): is:unread in:inbox | 1000 | 49.5 / 56.9 | 71.2 / 83.1 | 10 |
| search (grammar): "design review" | 855 | 28.7 / 32.1 | 43.6 / 45.0 | 10 |
| search (grammar): zebra quokka | 1000 | 87.3 / 187.6 | 114.1 / 126.2 | 10 |
| search (grammar): subject:launch -in:trash | 0 | 1.5 / 1.7 | 1.6 / 1.6 | 10 |
| search (plain FTS, 100 threads) | 100 | 3.0 / 3.3 | 4.8 / 5.2 | 20 |
| open 50-message thread, default expansion (perfgml1) | 50 | 0.2 / 0.3 | 0.6 / 0.6 | 20 |
| open 50-message thread, all bodies (perfgml1) | 50 | 0.4 / 0.5 | 1.4 / 2.7 | 20 |
| open 50-message thread, default expansion (perfwrk1) | 50 | 0.2 / 0.3 | 0.6 / 0.7 | 20 |
| open 50-message thread, all bodies (perfwrk1) | 50 | 0.4 / 0.5 | 1.4 / 1.5 | 20 |
| open 50-message thread, default expansion (perfhom1) | 50 | 0.2 / 0.2 | 0.5 / 0.6 | 20 |
| open 50-message thread, all bodies (perfhom1) | 50 | 0.4 / 0.5 | 1.2 / 1.3 | 20 |
| contact autocomplete "ann" | 15 | 0.8 / 1.1 | 1.7 / 1.8 | 20 |
| top contacts | 15 | 0.0 / 0.1 | 0.1 / 0.2 | 20 |
| contacts list (all with refs) | 30296 | 47.2 / 48.4 | 97.3 / 116.7 | 20 |
| contacts list (address book) | 4500 | 13.8 / 14.9 | 32.5 / 41.7 | 20 |
| calendars | 7 | 0.0 / 0.1 | 0.0 / 0.1 | 20 |
| calendar week: events.expand (both values) | 227 | - | 9.7 / 10.0 | 20 |
| calendar week: Event range query | 223 | 1.6 / 2.1 | 3.3 / 3.9 | 20 |
| calendar month: events.expand (both values) | 1104 | - | 19.3 / 23.3 | 20 |
| calendar month: Event range query | 369 | 2.1 / 2.6 | 4.4 / 5.2 | 20 |
| delta to render: star in inbox |  | 7.1 / 7.3 | 7.1 / 7.3 | 40 |

Notes on the rows:

- Since stage 0 the counts (`counts.get`) and the calendar (`events.expand`) are the
  engine's whatever the flag says; their SQLite column is the removed `ThreadCounts` SELECT,
  for scale. The grammar search runs in the engine on both values too (stage 1); the flag
  changes the messages join under its up to 1,000 result threads.
- SQLite's scroll p95 includes starting the background query agent, which exits after a few
  idle seconds and starts again on the next scroll.

## Stress profile

| Shape | rows | SQLite p50 / p95 ms | engine p50 / p95 ms | n |
|---|---:|---:|---:|---:|
| first page: unified inbox | 200 | 21.4 / 22.3 | 54.0 / 55.8 | 20 |
| first page: gmail inbox (label) | 200 | 12.0 / 15.1 | 30.3 / 36.1 | 20 |
| first page: imap inbox (folder) | 200 | 10.3 / 12.1 | 25.8 / 32.5 | 20 |
| first page: unified sent | 200 | 32.6 / 34.6 | 85.9 / 112.9 | 20 |
| first page: gmail all mail | 200 | 21.5 / 23.3 | 56.4 / 59.7 | 20 |
| first page: gmail label | 200 | 5.2 / 5.6 | 12.9 / 14.2 | 20 |
| first page: starred | 200 | 7.1 / 7.6 | 17.4 / 19.8 | 20 |
| first page: unread | 200 | 8.0 / 8.7 | 17.9 / 21.1 | 20 |
| scroll step: unified inbox |  | 4.6 / 13.1 | 6.9 / 8.6 | 84 |
| scroll step: gmail all mail |  | 7.7 / 12.2 | 13.0 / 15.4 | 84 |
| delta to render: star in inbox |  | 8.1 / 8.7 | 8.0 / 8.5 | 40 |

The other shapes match the profile's within noise.

## Where the time goes

A first page is two reads in a row: the 200 threads, then all messages of those threads for the
list's join (`features/thread-list/data-source.ts`). Split per read, unified inbox, p50 ms:

| Read | rows | answer | SQLite read + inflate | engine read + inflate | engine `idsOnly` |
|---|---:|---:|---:|---:|---:|
| threads | 200 | 162 KB | 2.5 + 0.8 | 5.4 + 0.4 | 3.7 |
| messages (profile) | 460 | 525 KB | 0.6 + 1.6 | 4.5 + 0.8 | 0.5 |
| messages (stress) | 3,701 | 4.2 MB | 3.8 + 13.0 | 41.8 + 6.2 | 3.7 |
| messages, unified sent (stress) | 5,792 | 6.6 MB | 5.0 + 19.6 | 62.9 + 9.5 | 5.6 |

- The SQL is not the cost: `idsOnly` reads take 1-6 ms. Over the socket each MB of model JSON
  costs about 9 ms (the engine parsing each row's `data` and writing the answer, the socket,
  `JSON.parse` of one line in the window); on SQLite about 4 ms (better-sqlite3 plus one
  `JSON.parse` per row). Inflation is cheaper from the engine's parsed rows.
- So the engine route costs about 11 ms per MB of first page against SQLite's 4, and a first
  page stays under 30 ms while its answers stay under about 2 MB: roughly 1,600 messages for
  200 threads, 8 per thread. Lists of long threads past that (notification threads, mailing
  lists Gmail threads together) miss the target; SQLite misses it at about 6 MB.
- The messages join is the cost the plan's thread `summary` (section 4.1, stage 5) removes:
  one read with a few fields per thread instead of every message's full JSON. Until then
  `"sqlite"` is the fallback for such profiles.
- The contacts app's list (30k contacts, 97 ms against 47) and 1,000-thread searches (20-80 ms
  more) pay the same per-MB cost. Neither has a 4.7 target; stage 8b's `contacts.list` and
  the `summary` include cover them.

## Stage 5: the thread summary route (2026-10-26)

Stage 5 part b adds `include: ["summary"]` on Thread `query` and live queries, so a first page
is one read of 200 threads with a few fields per thread instead of the threads plus every
message's JSON. `thin-perf.test.ts` measures it on the engine value only, as `query` and as a
live query's first window (`query.subscribe`), parsed and inflated. The desktop's list moves to
it in stage 6, so the rows above (the messages join) are still what the app does today.

First page p95, ms, over the eight lists:

| Route | Profile | Stress profile |
|---|---|---|
| threads + messages join (the app today) | 11.8-19.3 | 14.5-116.1 |
| `query` with `summary`, first 5b run | 5.4-13.3 | 6.2-32.8 |
| `query` with `summary`, final | 5.6-11.7 | 6.0-25.1 |
| live query with `summary`, final | 6.0-22.8 | 6.7-35.7 |

- The summary route meets the 30 ms target on both profiles. Its cost on the stress profile is
  reading the window's messages in the engine (unified sent: 5,792 messages, about 7 MB of
  `data` parsed into the few keys the summary needs), not the answer, which stays at
  0.5-0.6 MB.
- A live query's first window also counts the whole list for `total`: 5.6 ms for the unified
  inbox's 5,186 threads, 12.0 ms for unified sent's 10,462 on the stress profile (the count
  joins `ThreadCategory` to `Thread` and reads each thread's row). That is the gap between the
  two summary rows, and why unified sent and all mail miss 30 ms as live queries on the
  stress profile. Counting on `ThreadCategory` alone saves about a third (7.3 against
  11.1 ms); a cheap count needs `ThreadCounts`-style totals or an optional `total`, deferred.
- The first 5b run also showed the messages-join route slower than in stage 2 (3,701-message
  read 41.8 -> 58.5 ms p50, unified sent first page 85.9 -> 120.3 ms). It was noise on a loaded
  machine, not the read pool or the derived fields: on one build, over the socket from a plain
  client, the 3,701-message read takes 36.1-36.4 ms p50 and the 5,792-message one 57.9-58.5 ms
  with and without the pool and the derived fields (temporary switches, removed). In process,
  adding `hidden` and `warnings` to 5,890 messages takes 2.6 ms (0.4 us each); `warnings` now
  checks Reply-To first, which most messages lack. The final run is back near stage 2: the
  3,701-message read 43.5 ms p50 (stage 2: 41.8), the 5,792-message one 69.3 (62.9), unified
  sent's first page 94.5 / 116.1 (85.9 / 112.9). The other shapes match the tables above within
  noise; scroll steps p95 6.5-10.9 ms, delta to render 8.4-9.5 ms.

## Stage 6: the desktop's thread list on live queries (2026-10-26)

Stage 6 part a moves the desktop's lists to live queries: with reads on the engine,
`ThreadListDataSource` is one `query.subscribe` with `include: ["summary"]` and no messages
join, a scroll step is a `query.setWindow`, and a change reaches the list as `query.changed`
instead of a delta the window patches. `thin-perf.test.ts` now runs the list scenarios three
ways: SQLite, the engine as the app reads it now (the "engine" column), and the engine with
live queries off, which is the stage 2 route (rows marked "query + join"). Delta to render now
waits 100 ms before each star, so each change is measured on its own (see the last bullet).

p50 / p95, ms:

| Scenario | Profile: live + summary | Profile: query + join | Stress: live + summary | Stress: query + join |
|---|---|---|---|---|
| First page, 8 lists (p95 range) | 6.6-22.7 | 11.2-26.3 | 7.3-61.7 | 14.5-100.0 |
| First page, unified inbox | 15.2 / 22.1 | 14.3 / 26.3 | 24.2 / 27.4 | 59.0 / 62.6 |
| First page, unified sent | 16.5 / 22.0 | 17.9 / 22.2 | 29.5 / 61.7 | 89.2 / 100.0 |
| First page, gmail all mail | 19.5 / 22.7 | 14.2 / 14.7 | 26.6 / 28.6 | 55.0 / 58.1 |
| Scroll step, unified inbox | 12.1 / 13.9 | 7.2 / 10.5 | 11.3 / 14.8 | 5.5 / 6.8 |
| Scroll step, gmail all mail | 20.6 / 22.4 | 9.3 / 10.7 | 20.7 / 23.3 | 9.0 / 10.2 |
| Delta to render, star in inbox | 18.7 / 23.3 | 8.0 / 8.8 | 18.4 / 19.8 | 9.6 / 10.2 |

SQLite in the same runs: first pages 4.7-10.7 p95 (stress 5.6-35.6), scroll steps 10.1 and
13.0, delta to render 8.8 (stress 10.3).

- Every 4.7 target holds on the profile. On the stress profile the first pages are 1.5-3 times
  faster than the messages join and seven of eight meet 30 ms; unified sent misses at p95
  (p50 29.5, three of twenty samples at 61-80 ms), as its SQLite read does (35.6). Its cost is
  the live query's `total`, which counts the whole list (stage 5's section above).
- Scroll steps are slower than on the join route and still under the target: a
  `query.setWindow` answers the whole new 200-row window with summaries and its `total`, where
  the join route reads only the 25 rows that came into range. Gmail all mail, the longest list,
  pays most for the count. A window move that sends only the rows that entered it, or a
  `total` kept between moves, would bring it near the join route's; deferred.
- Delta to render on a live query is the engine re-running the window (ids and count, then the
  changed rows with summaries) and sending the steps, about 10 ms more than patching the row in
  the window. A change within 50 ms of the engine's last re-run waits out the rest of the 50 ms
  (re-runs are coalesced): back to back, star then unstar measured 24-66 ms in a smoke run.
  That is under the target for one change; a burst shows its last change within about 50 ms.

## Not measured

- Idle CPU per window during a sync burst (4.7 item 4): the profile engine syncs nothing.
  Windows take every delta on both values; what the flag changes is where a re-read goes.
- The 65 captured shapes one by one: `model_query_parity.rs` checks their results; the
  call sites above are the ones that read large results. Small reads (top contacts, calendars,
  a thread's messages) take under 3 ms on both values.
- More than one window, and the gateway (web) route.
