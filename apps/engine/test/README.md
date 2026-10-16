# Sync engine integration tests

Black-box tests that run a real `mailsync` binary against a mail server in a known state,
let it sync, optionally change things on the server or queue tasks on stdin, and then check
that the engine's database says what the server says. No C++ is compiled for these; the
engine is exercised exactly as the Mailspring client exercises it.

```
test/
  harness/        drives mailsync, reads its database/log/delta stream, runs scenarios;
                  servers/people.py is the fake Google OAuth + People API
  fakeimap/       an in-process, scriptable IMAP server with server "personalities"
  servers/        Dockerfiles + configuration: Dovecot 2.3.21, Cyrus IMAP 3.6, and the
                  CalDAV/CardDAV servers Radicale 3.8 and Baikal 0.10 (sabre/dav 4)
  scenarios/      one YAML file per scenario
  conformance/    proves the fake answers like Dovecot for everything mailsync sends
  tools/          ab.py (before/after outcome comparison, and `ab.py batch`: every case on
                  the Rust and C++ binaries with DB/delta diffs by headerMessageId, written
                  to docs/engine-ab-report.md; rules in ab_known.py), record_personality.py,
                  cyrus_server.py (a standalone Cyrus for the Mailspring client)
  docs/           adding-scenarios.md (workflow + gotchas), handoff-refactor-regression.md;
                  tasks/ holds write-ups of engine bugs the suite finds until they are fixed
  runs/           per-run artifacts (gitignored): engine log, DB, server transcript
  ab/             gitignored: baseline binaries and ab.py recordings (`ab/batch/` for `batch`)
```

Start with `docs/adding-scenarios.md` if you are here to add a test, and
`docs/handoff-refactor-regression.md` if you are here to check a refactor for regressions.

## Running

```bash
pip install -r test/requirements.txt   # pytest, pytest-xdist, pyyaml
python3 -m pytest test                 # quick tier: fake server only, no Docker, ~3 min
python3 -m pytest test --all           # both tiers, every server kind: ~12 min
python3 -m pytest test --slow          # slow tier only
python3 -m pytest test --servers fake -k qresync       # narrow by server kind / name
python3 -m pytest test -n 0 -k baseline-initial-sync   # serially, for debugging
python3 test/run.py test/scenarios/qresync-bulk-expunge-during-idle.yaml --server dovecot:qresync --keep
python3 test/conformance/compare.py -v                  # fake vs Dovecot, full diffs
python3 -m pytest test/test_service_fakes.py test/test_pim_oracles.py -n 0   # harness self-tests
```

Server kinds are `fake` (in-process) and, with Docker, `dovecot`, `cyrus`, `radicale` and
`baikal`; an image is built the first time a case needs it (a lock file serialises the
build across workers) and re-tagged whenever `servers/<kind>/` changes. The log
prints a line when a case starts and `[gwN] [ 42%] PASSED|FAILED <case>` when it ends; a run
is stuck only if no line appears for ~3 min (the slowest case takes ~140 s). The run ends
with a "scenarios needing attention" section: every FAILED / XFAIL / XPASS case, its reason
and its artifacts directory.

The binary comes from `MAILSYNC_BIN`, else `target/release/mailsync` at the repo root, else
the C++ engine's Linux cmake output (`legacy/mailsync-cpp/mailsync`), else Xcode's
DerivedData; `--mailsync PATH` overrides. Release builds refuse to start unless argv[0]
contains the product name ("tern" for the Rust engine, "mailspring" for the C++ one); the
harness launches through a `tern-mailspring-bin/` symlink so neither check bites.
Without Docker, Dovecot runs as a local process if `dovecot`/`doveadm` are installed
(`HARNESS_DOVECOT_MODE=local|docker`). `HARNESS_SERVERS=fake,dovecot` forces the set.

Artifacts go to `test/runs/session-<pid>/<scenario>-<server>/` (one pid per xdist worker;
`HARNESS_RUNS_DIR` overrides), and are kept for failed and xfailed cases: `report.txt` (what
happened, when), `config/mailsync-*.log` (engine log with every IMAP line, per thread),
`config/edgehill.db`, `server.log` (server transcript). `--keep` / `HARNESS_KEEP=1` keeps
passing runs too. `run.py --server kind:profile` uses the scenario's own entry for it.

### Tiers

The default run is the quick tier; `--slow` runs only the slow tier and `--all` both. A case
is in the slow tier (pytest marker `slow`) when it

- needs a Docker server kind (`dovecot`, `cyrus`, `radicale`, `baikal`, including the
  conformance probes and the DAV self-tests): these always carry `slow` and `docker(kind)`,
  and are skipped when Docker is missing;
- or its scenario says `slow: true` (or a reason), or `slow: {"fake:plain": true}` for one
  server. Use this for a fake-server case that seeds heavily or waits on engine timers and
  takes over ~20 s on its own, not only under a loaded parallel run.

`--servers dovecot` (any Docker kind) runs that kind's cases whatever the tier. The run ends
with a list of quick-tier cases over 20 s, the candidates for `slow: true`. Cases run longest
first, using `durations.json` (refresh it with `--record-durations`; a case missing from it
counts as 30 s), so one long case does not start last.

## How a scenario works

```yaml
name: qresync-bulk-expunge-during-idle
source: Mailspring-Sync PR #141          # the bug or behaviour this protects
servers: [{fake: dovecot}, {dovecot: qresync}]
mailboxes:
  INBOX: {messages: 200}
steps:
  - wait: quiescent
  - server.expunge: {mailbox: INBOX, uids: "140:195"}
  - wait: {quiescent: true}
  - sync: pass
expect:
  db_matches_server: {}
  counts: {INBOX: 144}
  stable: {passes: 1}
  running: true
```

Each `servers` entry becomes one pytest case. `mailboxes` populates the server before
mailsync starts (`messages:` is a count or `{count, html, attachment, thread, self_addressed,
age_days, ...}` for `harness/mailgen.py`; `flags:`; `duplicate_of: {mailbox, uids}` for
byte-identical copies). `setup:` steps run before mailsync starts.

The other services are populated the same way, before mailsync starts:

| key | meaning |
|---|---|
| `people: {groups: [{name}], contacts: [{name, email, groups} \| {count: n}]}` | contacts and user groups in the fake People API (needs `people: true` on a `fake` entry) |
| `calendars: {name: {create, display_name, components, events: [spec \| {count: n}]}}` | CalDAV collections and events (`default` is the seeded calendar; others are created with MKCALENDAR when `create: true`); event specs are `harness/davgen.py`'s: uid, summary, start (`"+2d"`, ISO), minutes, all_day, rrule, overrides, attendees |
| `addressbooks: {default: {contacts: [spec \| {count: n}], groups: [{uid, name, members: [emails or uids], version}]}}` | CardDAV cards; contact specs: uid, name, email(s), phone, org, version `"3.0"`/`"4.0"`; group cards are vCard 3 `X-ADDRESSBOOKSERVER-KIND:group` or vCard 4 `KIND:group` |
| `legacy_db: {wal, folders, messages, metadata, state, files, sql}` | an `edgehill.db` as the C++ engine leaves it, written before the first `--mode migrate` (`harness/legacy.py`); `wal: true` leaves the writes in the `-wal` file; a draft's `reply_to: <message key>` makes it a reply saved in that message's thread |
| `import_legacy: true` | run `--mode import-legacy` between migrate and sync, as the client does on every launch (also `restart: {import_legacy: true}`) |
| `slow: true` / `slow: {kind[:profile]: true}` | the case is in the slow tier (see "Tiers") |
| `xfail: reason` / `skip: reason` | the whole scenario waits on an engine feature: failures are reported as XFAIL (a pass as XPASS), or the case is skipped |

Steps:

| step | meaning |
|---|---|
| `wait: quiescent` / `wait: {quiescent: true, timeout: s}` | until the engine has nothing to do (see below) |
| `wait: {seconds: n}`, `wait: {log: regex}`, `wait: {task: label}` | |
| `wait: {log: regex, count: n, since: mark}` | until the log holds n matching lines, counting earlier processes (the log file survives `restart`); with `since`, only lines timestamped at or after the mark |
| `wait: {until: mark, plus: s}` | sleep until s seconds after the mark (wall time as the condition, e.g. an expiration passing) |
| `wait: {smtp_sent_copies: n, timeout: s}` | until a `smtp_sent_copy_delay` server has filed n sent copies |
| `wait: {counts: {mailbox: n}, timeout: s}` | until the local placement counts match, as `expect: counts`; the outcome, where a log line can arrive as an unparsed continuation |
| `sync: pass` | `wake-workers` on stdin, then wait for that pass to finish |
| `server.expunge / flags / move / copy / duplicate / append / create_mailbox / delete_mailbox / set_uidvalidity / set_uidnext / drop_connections / pause` | what another client does to the mailbox (`pause` changes nothing: with `at` and `delay` it holds one reply, as in `undo-before-remote-phase`); `at: before_fetch_body|idle_start|idle_tick|before_command` defers it to that protocol moment (fake only) |
| `at: {hook, session: foreground|background, command: regex, mailbox, delay: s}` | the same, narrowed to one connection (`foreground` = the one that has idled), one command line (`"^UID FETCH 1:\\*"`) and one selected mailbox; `delay` holds that connection's reply so another connection acts on the change first (`mid-pass-foreground-tombstone`) |
| `server.reject: {at: {hook: before_command, command: regex}, code, text}` | the hooked command is answered `NO [code] text` instead of being run (fake only; commands without literals), as in `move-rejected-by-server` |
| `at: {..., every: true}` | keep the hook armed instead of firing once, e.g. a folder whose STATUS fails on every pass (`orphan-sweep-with-unreadable-folder`) |
| `server.flags: {..., per_message: true}` | one STORE per UID, so HIGHESTMODSEQ advances once per message - how a modseq gap larger than one grows on a real server (used by `modseq-truncation`) |
| `client.task: {__cls: ChangeFolderTask, messages: {mailbox, uids}, folder: Archive}` | `Actions.queueTask` on stdin; `messages` resolve to engine ids (`{mailbox, uids}`, or `{header_message_ids: [...]}` for rows without a server placement such as a local draft), `threads: {mailbox, uids}` to their `threadIds`, `folder`/`labelsTo*` to Folder JSON, `sourceFolders: [paths]` to `sourceFolderIds` |
| `client.task` drafts | in `SyncbackDraftTask` / `SendDraftTask`, a `draft.files` entry with `content` is written where the client keeps draft attachments (`files/<id[0:2]>/<id[2:4]>/<id>/<filename>`, id lowercased) and `content` is dropped; `draft: {stored: <Message-ID>}` sends the draft the engine stores, with its body, as the client does for a saved draft |
| `client.undo_task: {of: label}` | queue the undo of a completed task from its stored data, as `UndoRedoStore` does: for a `ChangeFolderTask` the engine-written `undoPlacements` become `restorePlacements` and the original destination its `sourceFolderIds` |
| `client.need_bodies`, `client.wake` | the other stdin commands |
| `client.sync_calendar` | `{"type": "sync-calendar"}`: one calendars-only DAV pass on the `calendar` thread. Contacts sync once per process start (then every 75 min), so contact changes need a `restart` |
| `client.task` for PIM | `SyncbackMetadataTask` with `model: {threads\|messages: selector}` (modelId, modelClassName and modelHeaderMessageId filled in) and `value` where `{now_plus: s}` is an epoch `s` seconds ahead; `SyncbackEventTask` with `calendar: name, event: {ics: event spec}` (new) or `event: {uid, changes: {SUMMARY: ...}}` (edit, SEQUENCE bumped); `DestroyEventTask` with `events: [{uid}]`; `SyncbackContactTask` with `contact: {new: {name, email}}` or `contact: {uid\|email\|name, changes: {name, email}}`; `DestroyContactTask` / `ChangeContactGroupMembershipTask` with `contacts: [selectors]`; `SyncbackContactGroupTask` with `group: {new: true, name}` or `{name, rename}`; `DestroyContactGroupTask` with `group: {name}`. The task's JSON is built from the engine's stored row, as the client builds it from its model |
| `dav.add_event / update_event / delete_event / add_contact / update_contact / delete_contact / add_group / set_group_members / create_calendar / delete_calendar` | what another CalDAV/CardDAV client does: PUT with If-None-Match or If-Match, DELETE with If-Match, MKCALENDAR (`dav.update_event: {uid, changes: {SUMMARY: x}}`, `dav.update_contact: {uid\|email, changes: {FN: x}}`, `dav.set_group_members: {uid, members: [emails]}`) |
| `people.add_contact / update_contact / delete_contact / add_group / rename_group / delete_group / group_members / expire_sync_tokens` | what contacts.google.com does; `expire_sync_tokens` makes every issued sync token answer 400 "Sync token is expired" |
| `mark: name` | record the wall clock, for `changed_since` and `metadata_expiration: [{since}]` and `wait: {delta: {since}}` |
| `wait: {models: {Table: {where, count, match, absent}}, timeout: s}` | until the engine's rows match, as `expect: models` |
| `wait: {delta: {class, type, where, count, since}, timeout: s}` | until that many models arrived on deltas of that class and type (e.g. `metadata-expiration`, `ProcessAccountSecretsUpdated`) |
| `wait: {people_requests: [...], timeout: s}` | until the fake People API has seen those requests, as `expect: people_requests` |
| `server.smtp_hold` / `server.smtp_release` | queue self-addressed SMTP deliveries, then deliver them, so the INBOX copy of a sent message arrives after the Sent copy; `smtp_release` takes `at:` like any server step (`rules-ready-inbox-copy-first`) |
| `hold_engine: true` on a `server.*` step | SIGSTOP the engine while the step runs and SIGCONT after, so a slow server change (a 2300-message APPEND to Cyrus) lands as one batch rather than being split by a pass that wakes from the 120 s sleep halfway (`heavy-fetch-truncation-backlog`) |
| `force_scans: {}` | backdate `lastDeep`/`lastShallow` in the DB and wake (see Stopgaps) |
| `db.sql: [statements]` | write the stopped engine's database (only under `restart: {before: [...]}`), to reproduce a stored state whose producing path is unknown; every derived layer must be written consistently |
| `restart: {binary: path, before: [steps]}` | stop and relaunch on the same database, optionally with another build; `before:` runs steps while the engine is stopped (server state it did not watch happen) |
| `snapshot: name`, `assert: {...}` | mid-scenario checkpoints |

A scenario may also start on another build with a top-level `binary: ab/mailsync-0df7864`
(relative to `test/`) and `restart` onto the build under test (`rules-ready-pre-upgrade-messages`).
`--mailsync` still sets the build under test that a `restart` with no `binary:` lands on. The
harness runs `--mode migrate` on every launch, as the client does. The Rust engine never
upgrades a C++ database: it moves it to `edgehill.legacy.db*` and starts fresh
(`fresh-db-replaces-legacy`), so a C++ database is handed to it with `legacy_db:`, or by
starting on the C++ engine with `binary: cpp` and restarting onto the build under test
(`import-legacy-from-cpp-engine`). `cpp` is `MAILSYNC_CPP_BIN`, else the engine an
installed desktop app ships (`/Applications/Mailspring.app/.../app.asar.unpacked/mailsync`),
else `legacy/mailsync-cpp/mailsync`; the scenario is skipped when none exists.

A top-level `env: {NAME: value}` is added to the engine's environment on every launch, for
the knobs the engine reads from it: `ORPHAN_SWEEP_MAX_WAIT` (seconds; default one day) is how
long the orphan sweep waits for a folder that has not been fully scanned.

Expectations: `db_matches_server` (placements: every (folder, UID) on the server is a
message locally with the same Message-ID and tracked flags, and nothing local is missing on
the server), `counts`, `shown` (messages per folder as the client sees them: the
`Message.folders` snapshot, so a copy in flight counts in its pending destination), `server_counts`, `server_has: {mailbox: [Message-IDs]}` (those messages are in that mailbox on the server, whatever the engine believes; an entry `{message_id, flags: [...]}` also requires a copy there with exactly those tracked flags), `stable: {passes: N}` (N more passes over an
unchanged mailbox must not move a single placement - the flapping detector), `folder_status`,
`log_present` / `log_absent`, `log_count: {regex: n}` or `{regex: {min, max}}` (how many
log lines match, to bound a loop the engine should take a known number of times),
`deltas: {Message: {unpersist: 0}}`, `rules_ready: {total, messages: {Message-ID or subject:
{count, folders, fetched, metadata}}}` (the one-shot flag mail rules run on: no message carries
it on two deltas, across restarts, and each such delta carries the body; `folders` the
mailboxes that delta must list, `fetched` whether it is the delta that fetched the body),
`folder_roles: {path: role}`, `thread_labels: {roles: {label path: role or null}, threads:
[{mailbox, uids, has, lacks, server_has, server_lacks}]}` (Gmail labels on those messages'
threads by path, each carrying its Label row's role, and their X-GM-LABELS on the server),
`connection_error: {reported: true, cleared: true}`
(the ProcessState stream behind the client's offline state), `migrate_output: regex` (the
last `--mode migrate` stdout) or `{launch: first|last|n, present: [regexes], absent:
[regexes]}`, `mode_output: {mode: regex}` (a one-shot mode run before sync),
`smtp: {count, messages: [{recipients, headers: {name: value}, attachments: {filename:
content}}]}` (what the SMTP sink received, in order),
`unchanged_since: snapshot`, `running`, `exit`. Any expectation given as a mapping may carry
`xfail: reason` for a known engine bug: it is recorded, not failed, and reported as XPASS
once it starts passing.

Expectations for contacts, calendars, metadata and whole-database state (`harness/pim.py`):

| expectation | meaning |
|---|---|
| `models: {Table: {where, count, match, absent}}` | rows of a model table (Contact, ContactGroup, ContactBook, Calendar, Event, Message, Thread, File, Task) selected by their JSON: `{email: x}` matches `data.email`, falling back to the column; a value is a scalar, `{re}`, `{present}`, `{in}` or `{not}`; `count` is `n` or `{min, max}` |
| `dav_matches_db: {window: true}` | the DAV oracle: every VEVENT collection is a Calendar, every in-window VEVENT (master and each RECURRENCE-ID) is one Event with the server's etag, href and SUMMARY, every card is a Contact keyed by UID with its etag, href, FN and first EMAIL, group cards are hidden contacts plus a ContactGroup with the same members, each address book is a carddav ContactBook, and nothing local is missing on the server |
| `people_matches_db: {}` | the People oracle: one `gpeople` Contact per person (by resourceName) with its name and email, one ContactGroup per user group with its members, no system groups, one gpeople ContactBook |
| `dav_has: {events, no_events, contacts, no_contacts, groups: {name: [emails]}}` / `people_has: {contacts, no_contacts, groups, no_groups}` | the service's state, whatever the engine believes: what proves a task's remote phase wrote it |
| `people_requests: [{method, path: regex, query: {param: value \| true \| false}, status, count \| min \| max}]` | what the engine asked the People API (e.g. that an incremental pass sent its syncToken) |
| `tasks: {label: {status, error: null \| key regex}}` | how a queued task ended; `error` matches the stored SyncException key (`etag-conflict`) |
| `metadata: [{threads\|messages: selector, plugin, value: {subset}, expiration: bool, absent: bool}]` | the model's metadata entry and its ModelPluginMetadata row |
| `metadata_expiration: [{threads\|messages: selector, plugin, count, since: mark}]` | how many `metadata-expiration` deltas carried the model; none may arrive before the expiration it names |
| `changed_since: {mark, Table: {key, where, persist: [keys], unpersist: [keys]}}` | exactly which models were streamed after a `mark`, by `key` (default `id`): what proves a pass was incremental |
| `sql: [{db: main\|legacy\|<file>, query, equals}]` | a query against the engine's database, `edgehill.legacy.db`, or another database file in the config dir such as `edgehill.legacy-imported.db` (read from a copy, so the check never checkpoints its WAL) |
| `config_files: {present, absent, empty}` | paths under `CONFIG_DIR_PATH`; `empty` directories may also be missing |

**Invariants** run after every scenario's expectations, on every server, without being
listed (`harness/invariants.py`). Once the engine is quiescent they recompute each derived
layer of the engine's state from the stored layer below it, so a bug is reported once, where
it starts: `message_snapshot` (`Message.folders` / `labels` from `MessageFolder` rows),
`message_flags` (message unread / starred / draft from its rows, JSON vs indexed columns),
`thread_refcounts` (thread folder and label `_refs` / `_u`, unread / starred, inAllMail from
its messages' snapshots), `thread_categories` (`ThreadCategory` from the thread's arrays),
`thread_counts` (`ThreadCounts` from `ThreadCategory`), `orphans` (`MessageOrphan` lists
exactly the messages with no `MessageFolder` row, which is how the end-of-pass sweep finds them),
`contacts` (Contact columns from its JSON, one ContactSearch row each, `gis` equal to its
ContactContactGroup rows, join rows naming existing contacts and groups, the book existing),
`contact_groups` (columns, book), `events` (columns, the calendar existing, one row per
(calendar, UID, RECURRENCE-ID), EventSearch without dangling or duplicate rows), `calendars`
(one row per path) and `plugin_metadata` (ModelPluginMetadata equal to the metadata entries
of every Message and Thread whose value is not null, `{}` or `[]` (nlohmann `size() != 0`, so
`false` and `0` count), with `expiration` taken from `value.expiration`).
A scenario that trips one because of a known engine bug says so rather than skipping it
silently: `expect: {invariants: {skip: [thread_counts]}}` with a comment naming the bug,
`invariants: {xfail: reason}`, or `invariants: false` to disable all of them. They also work
mid-scenario as `assert: {invariants: {}}`.

**Quiescence** is inferred, since the engine has no "done" signal: every Folder's
`localStatus.busy` is false and `syncedMinUID <= 1`; the background thread's last log line
is `Sync loop complete.` and older than the settle window (it loops without sleeping while
there is more to do); the foreground thread's last IMAP command is `IDLE`; and no delta has
arrived within the settle window.

**The oracle.** `harness/db.py` is the only code that knows the engine's schema. It turns
`Message.remoteFolderId/remoteUID` (or `MessageFolder` rows, once they exist) into
`{folder: {uid: Placement}}`, and `Server.truth()` reads the same shape back from the server
over IMAP. Everything else compares those two, which is what lets the scenarios survive the
placements refactor: only `placements_from_db` changes.

## The fake server, and keeping it honest

`fakeimap/` is a stateful IMAP4rev1 server with CONDSTORE, QRESYNC, IDLE, UIDPLUS, MOVE,
ENABLE, ID, NAMESPACE, SPECIAL-USE, STARTTLS and the Gmail extensions (plus a small SMTP
server), driven either in-process (the
harness calls `store.expunge(...)` and the notifications flow to connected sessions exactly
as they would from another client) or at protocol hooks. Sequence numbers are per session;
a session is told about an expunge once; modseqs behave like Dovecot's.

A flaw in the fake would become a "must support" condition on the engine, so the rules are:

1. **The default personality (`dovecot`) is Dovecot 2.3.21**, and `conformance/` proves it.
   `conformance/probes.py` sends every exchange mailsync performs (session setup, LIST /
   STATUS, SELECT / EXAMINE, header and body FETCH shapes, CHANGEDSINCE with and without
   VANISHED, STORE incl. `.SILENT`, COPY / MOVE / APPEND, EXPUNGE, CREATE / RENAME / DELETE,
   and what an idling session is told) to both servers from identically populated state and
   diffs the normalized responses. 56 comparisons currently match exactly; a difference is a
   failing test unless it is listed in `ALLOWED_DIFFERENCES` with a reason. Run it after any
   change to `fakeimap/`.
2. **Every deviation from that baseline is a named quirk on a `Personality`** in
   `fakeimap/personalities.py`, with a citation: the PR, forum thread, RFC section or upstream
   commit that shows a real server doing it. Personalities whose exact wire format was
   reconstructed from reports rather than a captured transcript say `NEEDS-RECORDING`; a
   scenario may exercise the engine's branch for them, but their text should be replaced
   with output from a real session (`tools/record_personality.py`, to be written) before
   anything is asserted about that text.
3. **Generated messages are unique** (Message-ID, subject, date) unless a scenario asks for
   duplicates via `duplicate_of` / `server.duplicate`. The first version of this harness
   gave every mailbox `<msg{uid}@example.test>`, so INBOX's message 1 and Archive's message 1
   collapsed onto one row and flapped between folders - a fake-server artefact that looked
   exactly like a real bug.

Things learned from Dovecot while building the conformance suite, all now modelled:

- Attribute order in unsolicited/STORE FETCH responses is `UID`, `MODSEQ`, `FLAGS`; a
  `.SILENT` store still reports `MODSEQ` when CONDSTORE is enabled; a STORE that changes
  nothing reports nothing and does not bump the modseq.
- `UID FETCH ... (CHANGEDSINCE n VANISHED)` reports expunges this session has not yet been
  told about as a plain `* VANISHED`, then everything expunged since `n` as
  `VANISHED (EARLIER)` - **including on a repeat of the same request**. Dovecot does not
  remember what a session was told. (The first harness assumed the opposite.)
- The tagged OK of a command whose response carried a plain VANISHED carries
  `[HIGHESTMODSEQ n]`; after IDLE it is sent untagged.
- Several appends are reported as one `EXISTS`; `RECENT` is per session and only sent when
  the session's count changed; `EXAMINE` reports but does not claim `\Recent`.
- `CREATE a/b` creates `a` as `\Noselect`; deleting the last child deletes it again.
- IDLE notifications for a single change arrive ~0.5 s later; a burst of changes within a
  few milliseconds is reported only partly during IDLE and the rest after `DONE`. With
  maildir the default `mailbox_idle_check_interval` (30 s) is not what causes this.
- **A session's FETCH is answered from its own view.** If another session expunged messages
  since this session last synced, `UID FETCH 1:*` still returns those messages, and the
  `EXPUNGE` / `VANISHED` lines come *after* the FETCH data. `STATUS` on the selected mailbox
  is likewise stale (Dovecot marks it `[CLIENTBUG]`). mailsync's background connection keeps
  the last folder it touched selected, so a deep scan right after a deletion sees the old
  message set and only converges on the following pass (`plain-expunge-found-by-deep-scan`
  records this as an xfail). The fake models this (`probe_stale_fetch`).

## Server kinds

- `fake:<personality>` - `dovecot` (baseline), `plain` (no CONDSTORE/QRESYNC: the deep-scan
  branch), `proton-bridge`, `gateway-duplicate-list`, `netease-coremail`, `courier`, `outlook`,
  `icloud`, `gmail`, `yahoo` (permuted COPYUID), `cyrus-vanished` (Dovecot plus Cyrus's
  VANISHED clipped at `*`, cyrus-imapd #6071), `yahoo-messagelimit` (enforces
  MESSAGELIMIT=50 on UID FETCH, RFC 9738), `inbox-only` (no SPECIAL-USE, only INBOX
  configured: roles by name), `hmailserver` (STATUS reports UIDNEXT 0).
  `fake:dovecot-without-condstore-qresync` style names strip capabilities.
  Hostname-gated engine behaviour (iCloud, Outlook) needs the account's `imap_host` to
  resolve to 127.0.0.1; a scenario declares `{fake: icloud, imap_host: imap.mail.me.com}`
  and is skipped with instructions unless `/etc/hosts` maps it.
  `{fake: gmail, people: true}` makes the account a Google one: OAuth only (refresh token,
  no password; IMAP accepts exactly the access tokens the fake token endpoint issued, over
  `AUTHENTICATE XOAUTH2`), contacts from the fake People API (`harness/servers/people.py`:
  paging, sync tokens, tombstones with `metadata.deleted`, expired tokens, etag checks on
  updateContact, contact-group membership), and a CalDAV stub with no calendars for the
  Gmail calendar pass. `people_options: {rotate_refresh_token: true}` rotates the refresh
  token on the first grant; `expires_in` sets the token lifetime (a string by default).
- `radicale:default`, `baikal:default` - an IMAP account whose provider also serves CalDAV
  and CardDAV: the in-process fake IMAP (`imap: <personality>`, default `dovecot`) plus
  Radicale 3.8.3 (`servers/radicale/`, filesystem storage, `/test/calendar/` and
  `/test/contacts/`) or Baikal 0.10.1 on sabre/dav 4 (`servers/baikal/`, SQLite,
  `/dav.php/calendars/test/default/`, `/dav.php/addressbooks/test/default/`, a scheduling
  inbox/outbox in the calendar home). Both authenticate the IMAP user `test` / `pass` with
  Basic auth, answer `.well-known/caldav|carddav` with a redirect, support sync-collection,
  ctags and If-Match (412 on a stale etag). Both images ship with one calendar and one
  address book, which scenarios call `default`. `dav.log` holds the server's log. Docker only.
- `dovecot:<profile>` - `qresync`, `plain` (capability override, as the #140 control run),
  `no-move` (`plain` without MOVE: COPY + `\\Deleted` + expunge),
  `proton-like` (plain + `\All` "All Mail"), `tls` (self-signed, implicit TLS), `sdbox`.
- `cyrus:<profile>` - Cyrus IMAP 3.6.1 (Debian bookworm's `cyrus-imapd`, `servers/cyrus/`),
  the server Fastmail runs: its own CONDSTORE/QRESYNC, MOVE/COPYUID and SPECIAL-USE, and
  `expunge_mode: delayed` as Fastmail configures it (invisible over IMAP: expunged messages
  never reappear). Profiles: `fastmail` (default: NAMESPACE `(("" "/"))`, i.e.
  `altnamespace` + `unixhierarchysep`, folders at the top level, as a live Fastmail account
  shows), `default-ns` (Debian's out-of-the-box `INBOX.`-rooted folders with `.`; no major
  provider is known to use it, so only four scenarios list it), `plain` (`fastmail` with
  `suppress_capabilities: CONDSTORE QRESYNC`), `no-move` (`fastmail` with
  `suppress_capabilities: MOVE`). One container per run (~3 s); the image is
  built on first use and re-tagged whenever `servers/cyrus/` changes. Docker only.
  - **Namespace.** Scenarios keep their flat names. Under `default-ns` the adapter maps
    `Archive` to `INBOX.Archive` for every server operation, and `Server.scenario_name`
    maps LIST output and the engine's `Folder.path` back (`ScenarioRun._named`), so
    expectations, `counts`, task folders and reports read the same on every server kind
    while the engine sees the real `INBOX.` paths. Both mappings are the identity elsewhere.
  - **UID space** uses Cyrus's tools in the container: `set_uidvalidity` drops the
    mailbox's `cyrus.index` and `reconstruct`s it (a fresh time-based UIDVALIDITY, UIDs
    kept, flags restored over IMAP; the requested value is ignored), then drops every
    connection as Dovecot's `mailbox update` does - a session that had the mailbox selected
    otherwise answers `NO Mailbox does not exist` to everything until it re-SELECTs;
    `set_uidnext` puts a message file `<n-1>.` in the spool, `reconstruct -f`, and
    expunges it; `drop_connections` SIGTERMs every `imapd`, which closes the socket
    without a `BYE`.
  - `server.log` is Cyrus's per-user telemetry log: every session's full protocol
    transcript, the engine's and the harness's own, one `===== imap-<pid>` block each.
  - The Dovecot-only `proton-like` profile has no Cyrus counterpart (Cyrus has no `\All`
    mailbox short of an exotic config), so `proton-all-mail-duplicates` does not list it.

**Endpoint overrides.** The engine reaches these services through environment variables
the harness sets on every launch; the engine reads them in place of its compiled-in hosts:

| variable | replaces | set by |
|---|---|---|
| `MAILSYNC_GOOGLE_TOKEN_URL` | `https://www.googleapis.com/oauth2/v4/token` | `people: true` |
| `MAILSYNC_GOOGLE_PEOPLE_ROOT` | `https://people.googleapis.com/v1/` | `people: true` |
| `MAILSYNC_GOOGLE_CALDAV_ROOT` | `https://apidata.googleusercontent.com` (the Gmail calendar host) | `people: true` |
| `MAILSYNC_CALDAV_HOST`, `MAILSYNC_CARDDAV_HOST` | the DNS SRV lookup for the account's domain (`host[:port]`); discovery then runs from `.well-known` | `radicale`, `baikal` |

**Where the PIM scenarios expect more than C++ does.** These scenarios fail on a literal
port of the C++ in these places; the engine has to do what the scenario says.

| scenario | expects | C++ |
|---|---|---|
| dav-create-update-delete | a new contact or group card is PUT to `<collection>/<uid>.vcf`, and the card's UID is the Contact id | POSTs to the collection (`DAVWorker::writeAndResyncContact`) and gives a new group a random UID (`performLocalSyncbackContactGroup`) |
| google-people-incremental-sync | a connection with `metadata.deleted: true` is a tombstone and removes the contact | checks a top-level `deleted` that the People API does not send, so it saves a blank contact (`GoogleContactsWorker::run`) |
| dav-etag-conflict | a 412 on a contact PUT and on an event DELETE becomes `etag-conflict`, as for an event PUT | maps 412 only for an event PUT (`writeAndResyncEvent`); plan section 6 has the rule for all three |

The `contacts` invariant checks `gis` against ContactContactGroup only between a contact and
a group that both exist: C++ leaves `gis` naming a removed group (ContactGroup::afterRemove)
and join rows naming a removed contact (Contact::afterRemove), and the scenarios accept that.
google-people-sync-token-expired follows C++: the 400 clears the token and fails the pass,
the re-list waits for the next contact pass, and a full listing does not remove contacts it
no longer returns.

## Scenarios

Every scenario that lists a Dovecot profile also lists `cyrus:fastmail` (`cyrus:plain` where
it lists `dovecot:plain`), except `proton-all-mail-duplicates`; four also list
`cyrus:default-ns`.

| scenario | protects | servers |
|---|---|---|
| baseline-initial-sync | ingestion, flags, threads, attachments, no churn | all |
| condstore-initial-sync-large | #140 chunked walk on QRESYNC (2500 msgs) | fake, dovecot |
| sparse-uid-space | #140 review: UIDNEXT 20001 with 1100 msgs; 4500 over a hole | fake, dovecot |
| qresync-bulk-expunge-during-idle | #141 wide VANISHED during IDLE | fake, dovecot |
| qresync-wide-vanished-range | #141 bounded-range query, ghost at range end | fake, dovecot |
| qresync-two-expunges-during-idle | #141 second VANISHED line | fake, dovecot |
| qresync-vanished-during-body-fetch | #141 VANISHED on a non-IDLE command | fake |
| plain-expunge-found-by-deep-scan | deep-scan deletion detection | fake, dovecot |
| remote-move-to-archive | move followed once, no unpersist | fake, dovecot |
| remote-flag-changes | flags via IDLE and CHANGEDSINCE | fake, dovecot |
| new-mail-during-idle | EXISTS during IDLE, folders nobody idles on | fake, dovecot |
| self-addressed-inbox-and-sent | placements: same bytes in INBOX and Sent | fake, dovecot |
| o365-duplicate-sent-copies | placements: identical copies at adjacent UIDs | fake, dovecot |
| proton-all-mail-duplicates | #137 `\All` skip | fake, dovecot |
| gateway-duplicate-list-entries | #139 duplicate LIST lines | fake |
| netease-coremail-id-before-select | #121 ID before SELECT (Coremail greeting), STATUS without UIDNEXT | fake |
| uidvalidity-change | UIDVALIDITY remap | fake, dovecot |
| two-folders-identical-messages | #140 non-converging gap scan must settle | fake, dovecot |
| client-task-move | ChangeFolder/Starred/Unread tasks | fake, dovecot |
| client-task-trash-and-expunge | Trash + ExpungeAllInFolder | fake, dovecot |
| connection-dropped-during-idle | reconnect after the server drops connections | fake, dovecot |
| connection-dropped-during-command | a connection closed without BYE mid-command reads as offline, and the retry reconnects | fake, dovecot, cyrus |
| send-draft | SendDraftTask over SMTP, Sent copy, self-addressed delivery | fake (+smtp) |
| gmail-labels | All Mail + X-GM-LABELS views, webmail archive/label/star, trash task | fake |
| remote-move-destination-scanned-first | placements: move seen "present in B" before "gone from A"; never deleted/re-created | fake ×2, dovecot |
| trash-message-with-two-placements | placements: trash from Inbox takes the Sent copy too; a plain archive does not | fake ×2, dovecot |
| folder-deleted-with-copies-elsewhere | a folder deleted by another client or DestroyCategoryTask: its only-copies removed, copies elsewhere kept | fake, dovecot, cyrus |
| flag-change-on-second-placement | placements: per-placement flags on the Sent copy of a self-addressed message | fake ×2, dovecot |
| gmail-send-and-labels | Gmail: no Sent APPEND on send, one All Mail placement; ChangeLabelsTask keeps one placement | fake |
| heavy-fetch-truncation-backlog | #140 1024-header truncation and draining-backlog back-off | fake, dovecot |
| modseq-truncation | CHANGEDSINCE gap > MODSEQ_TRUNCATION_THRESHOLD bounds the request to the newest UIDs | fake, dovecot |
| mid-pass-foreground-tombstone | foreground VANISHED while the background's stale FETCH of INBOX is in flight; no resurrection | fake ×2 |
| trash-two-placements-without-uidplus | trash of INBOX+Sent copies without COPYUID (dest-fetch fallback) | fake |
| undo-move-restores-placements | sourceFolderIds move of one copy, and trash of an unread and a read copy, each undone via restorePlacements to the copy's own folder | fake ×2, dovecot ×2 |
| undo-before-remote-phase | undo queued while the move's MOVE is held: the undo's marker survives the move's commit | fake ×2, dovecot |
| move-into-folder-holding-a-copy | move into a folder that already holds a copy leaves two there; undo returns the added one | fake ×2, dovecot |
| mark-read-fans-out-to-all-placements | ChangeUnreadTask by threadIds hits every placement | fake ×2, dovecot |
| uidvalidity-change-large-mailbox | #140 truncated UIDVALIDITY rebuild re-loops (2 500 msgs) | fake ×2 |
| synced-draft-destroy (+ -courier) | DestroyDraftTask on a server-synced draft and a local UID-0 draft | fake ×2, dovecot |
| remote-move-while-task-in-flight | another client moves a message's only copy while a star task holds its syncedAt lock; the new copy is recorded, no unpersist | fake ×2, dovecot |
| move-rejected-by-server | MOVE answered NO [OVERQUOTA]: the copy shows in INBOX again and the syncedAt lock is released | fake ×2 |
| orphan-sweep-with-unreadable-folder | a folder whose STATUS always fails delays the orphan sweep by ORPHAN_SWEEP_MAX_WAIT instead of disabling it | fake |
| orphan-sweep-waits-for-initial-walk | an initial walk that moves down every pass holds the orphan sweep past ORPHAN_SWEEP_MAX_WAIT | fake |
| rules-ready-incoming | rulesReady once per incoming message, on its body delta; Junk never, until moved to INBOX; Sent never; archive and back does not repeat it | fake ×2, dovecot, cyrus |
| rules-ready-self-addressed-send | nothing for mail sent to others; self-addressed mail gets it when the INBOX copy arrives, after a held delivery | fake ×2, dovecot, cyrus (+smtp) |
| rules-ready-inbox-copy-first | INBOX copy recorded before the send path stores the body; the flag survives a coalesced metadata save | fake ×2 (+smtp) |
| rules-ready-across-restart | engine restarted between the Sent copy and the INBOX copy: still exactly one | fake, dovecot, cyrus (+smtp) |
| rules-ready-pre-upgrade-messages | messages ingested by the pre-rulesReady build never get it | fake, dovecot, cyrus (starts on `ab/mailsync-a83cc9a`) |
| rules-ready-gmail | Gmail: \\Inbox mail once, \\Sent-only never, a sent message once when filed under \\Inbox | fake |
| sent-copy-saved-late | Exchange files its Sent copy 6.5 s after SMTP: the late copy is removed and the next send waits for it | fake (+smtp) |
| sent-copy-multisend | tracked send: one server copy per recipient, all deleted, one untracked copy kept | fake (+smtp) |
| sent-copy-cleanup-store-refused | a late copy whose delete is refused keeps its placement and is removed on the next pass (CONDSTORE) | fake (+smtp) |
| sent-copy-multisend-late | as above with the per-recipient copies filed after the engine stopped waiting | fake (+smtp) |
| sent-copy-bcc | Bcc to self: the late Bcc-less Sent copy is removed; the INBOX and Sent copies are one row | fake (+smtp) |
| sent-copy-send-interrupted | engine killed during the Sent poll: on relaunch the draft row goes and nothing is resent | fake (+smtp) |
| sent-copy-late-after-window | a late copy hours after the send is still removed; without UIDPLUS too | fake (+smtp) |
| move-without-move-keeps-other-deleted | no-MOVE fallback expunges only the moved UID; another client's `\\Deleted` message survives | fake, dovecot, cyrus |
| move-without-move-connection-drop | connection lost after the server ran UID EXPUNGE: no plain EXPUNGE on reconnect | fake |
| folder-roles-by-localized-name | roles by name without SPECIAL-USE: modified UTF-7 names, Borradores, `\\Archive` | fake |
| status-uidnext-zero-new-mail | STATUS UIDNEXT 0 (hMailServer): new mail found from the message count on the next pass | fake |
| list-refused-transiently | O365 `NO Server Unavailable` to LIST: offline and retried, not a crash | fake |
| fresh-db-replaces-legacy | a C++ database (writes still in its WAL) is moved to `edgehill.legacy.db*`, a fresh V12 is created, `files/` emptied, "Running Migration" printed; nothing imported; no migration on the next launch | fake |
| import-legacy-carries-metadata | `--mode import-legacy` carries snooze/reminder metadata (re-keyed by headerMessageId), a local draft with body and file, and `_State` cursors | fake |
| import-legacy-from-cpp-engine | the same from a database the C++ engine wrote (`binary: cpp`): the draft's attachment survives migrate, metadata attaches as sync ingests mail, the imported snooze and send-later fire as `metadata-expiration`; People sync tokens are not carried | fake (slow) |
| import-legacy-twice | the imported draft sends with its attachment; later launches' imports change nothing (the sent draft stays sent, no metadata row duplicated) | fake +smtp (slow) |
| import-legacy-without-legacy-db | no legacy database: success, nothing written | fake |
| metadata-expiration-snooze | `thread-snooze` expiration streams the Thread as `metadata-expiration` once due, never early, never for other threads, never after the client clears it | fake |
| metadata-expiration-send-later | `send-later` on a local draft expires while the engine is stopped and is streamed after relaunch; metadata without an expiration never is | fake |
| google-people-initial-sync | OAuth-only Gmail account: refresh-token rotation reaches the client, 432 contacts in pages of 400, user groups with members, system groups skipped | fake (people) |
| google-people-incremental-sync | after a relaunch the stored sync tokens fetch only what changed, including `metadata.deleted` tombstones | fake (people) |
| google-people-sync-token-expired | an expired sync token falls back to a full listing that also removes contacts deleted meanwhile | fake (people) |
| google-people-contact-tasks | contact and contact-group tasks write through createContact / updateContact (etag) / deleteContact / contactGroups / members:modify | fake (people) |
| dav-initial-sync | RFC 6764 discovery, VEVENT collections only, the sync window, recurring master + override, vCard 3/4 contacts, group cards | radicale, baikal |
| dav-incremental-sync | `sync-calendar` pass applies exactly the changed events; contacts after a relaunch; unchanged collections send nothing | radicale, baikal |
| dav-create-update-delete | event, contact and group tasks PUT/DELETE with the right preconditions and keep the stored etags current | radicale, baikal |
| dav-etag-conflict | writes with a stale etag get 412 and fail as `etag-conflict` without overwriting the other client's change | radicale, baikal |

Known engine failures are marked `xfail` in the scenario with the reason; `pytest -rxX`
lists them and an `XPASS` line means the marker can be removed. Each open one gets a write-up
in `docs/tasks/` (symptom, failing scenario, how it was found, likely cause, definition of
done) that is deleted when the fix lands. As of 2026-09-22 there are none: the suite found
five engine bugs on 2026-09-19/20 (two VANISHED-accumulator segfaults, the `\\All` mailbox
never clearing `busy`, and two consequences of Dovecot's stale per-connection view) and all
five were fixed within two days (`6c1395e`, `4c25380`, `c5619a8`, `5080fb2`). The invariants
found a sixth on 2026-09-22, both workers applying one server change's thread delta to a
message loaded outside the transaction, fixed the same day.
Bringing up the Cyrus server kind on 2026-09-24 found one latent weakness with no failing
scenario: `docs/tasks/cyrus-stale-selected-mailbox.md` (a connection whose selected mailbox
was replaced keeps getting `NO` and never re-SELECTs; theoretical on Fastmail).

Scenario timing rule: after a server-side change on a QRESYNC server, wait for the engine to
receive it (`wait: {log: "recv \\* VANISHED"}`) before forcing a pass; Dovecot delivers
IDLE notifications ~0.5 s after the change and a `wake-workers` that interrupts IDLE first
defers the notification to the next cycle.

## Live Cyrus server

`tools/cyrus_server.py` runs the harness's Cyrus image as a long-lived container
(`mailsync-cyrus-live`) with a persistent test account, for pointing the Mailspring client
at by hand:

```bash
python3 test/tools/cyrus_server.py start     # create + seed on first run, else resume
python3 test/tools/cyrus_server.py status    # ports, credentials, LIST as the server shows it
python3 test/tools/cyrus_server.py stop      # stop, keeping the mail; `rm` deletes it
```

It uses the `fastmail` profile. The first `start` creates INBOX, `Sent`, `Drafts`, `Trash`,
`Archive` and `Junk` (SPECIAL-USE, top level, `/` separator) and seeds 20 messages: 14 in INBOX (5 unread,
2 flagged), 3 in Archive, and 3 self-addressed ones with a copy in both INBOX and Sent.
`start` also runs the harness's SMTP sink (`fakeimap/smtp.py`) as a detached process,
because Mailspring's account setup verifies SMTP; mail to `test@example.test` is delivered
into the Cyrus INBOX over IMAP, as Fastmail delivers self-addressed mail, and everything
else is accepted and dropped. Its log is `runs/cyrus-live/smtp.log`.

In Mailspring choose "IMAP / SMTP Setup" and enter:

| | |
|---|---|
| email | `test@example.test` |
| IMAP server / port / security | `127.0.0.1` / `1143` / none (allow insecure) |
| IMAP username / password | `test` / `pass` |
| SMTP server / port / security | `127.0.0.1` / `1025` / none (allow insecure) |
| SMTP username / password | `test` / `pass` |

The Cyrus admin is `cyrus` / `admin` (e.g. to `SETACL` or create other users over IMAP), and
`docker exec -u cyrus mailsync-cyrus-live /usr/lib/cyrus/bin/<tool>` reaches `reconstruct`,
`mbpath`, `cyr_expire` and friends. `--imap-port` / `--smtp-port` change the ports.

## Stopgaps and next steps

- **Scan intervals.** `DEEP_SCAN_INTERVAL` (10 min), `SHALLOW_SCAN_INTERVAL`, the CONDSTORE gap
  scan (24 h) and the 120 s worker sleep are compile-time constants. `sync: pass` covers the
  sleep via `wake-workers`; `force_scans` covers the rest by zeroing `lastDeep`/`lastShallow`
  in `Folder.localStatus` (a write to a database the engine owns - safe in practice because
  `syncNow` re-reads folders each pass, but a hack). Environment-variable overrides in the
  engine will replace it.
- **Gmail.** `fakeimap.store.GmailStore` models INBOX / Sent Mail / Starred / Important /
  Drafts / user labels as views of `[Gmail]/All Mail` with per-view UIDs, label changes as
  EXISTS/EXPUNGE in the affected views, deletion-from-a-view as label removal, deletion from
  All Mail as a move to Trash, and `X-GM-LABELS` excluding the selected view's own label.
  Labels are sent as quoted astrings (`"\\Inbox"`), which is what libetpan parses; the
  exact quoting Gmail uses is NEEDS-RECORDING. Unsolicited FETCH responses include
  `X-GM-LABELS`, which real Gmail may not do (also NEEDS-RECORDING).
- **SMTP.** `fakeimap/smtp.py` (AUTH PLAIN/LOGIN, optional Postfix-style HELO rejection,
  delivery of self-addressed mail back into INBOX) is enabled per server spec with
  `{fake: dovecot, smtp: true}`, and on Dovecot and Cyrus the same way, where it delivers by
  APPENDing to INBOX over IMAP; `--mode test` and the EHLO fallback (#135) have no scenario yet.
  `{fake: gmail, smtp: true, smtp_sent_copy: "[Gmail]/Sent Mail"}` also files every submitted
  message under the named mailbox, as Gmail's submission service saves sent mail under `\Sent`
  by itself - the copy the engine's send path looks for before it APPENDs its own
  (`gmail-send-and-labels`). `smtp_sent_copy_delay: s` files it that long after the 250, as
  Exchange Online does (`sent-copy-*`).
- **Recording real providers.** `tools/record_personality.py HOST PORT USER --name NAME`
  captures greeting, capabilities, NAMESPACE, ID, LIST, STATUS/SELECT and FETCH shapes from a
  real account (no message content) into `fakeimap/recordings/`; use it to replace every
  `NEEDS-RECORDING` personality field with the real text.
- **Live accounts.** `Server.truth()` works against any IMAP server, so a pre-merge smoke
  against real accounts (run N passes, assert `stable`) needs only credentials plumbing.
