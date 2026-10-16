# Engine socket protocol, CLI and single engine process

Draft, 2026-10-16; S8, the integrations order and the phase B read API confirmed by the
user the same day. Roadmap steps 1 and 2 (memory: tern-roadmap-after-parity): replace the
engine's stdin/stdout channel with a local socket, ship a CLI client as standalone binaries,
then run one engine process for every account. Engine paths are relative to the repo root;
desktop paths to `apps/desktop/app/`.

The frozen client contract (engine-port-plan D5) held until parity and is lifted by this
plan. The stdio contract in engine-port-plan section 3 stays as the legacy transport for as
long as the C++ engine is selectable and the A/B harness needs it (S0).

## 0. Proposed decisions

| # | Decision |
|---|---|
| S0 | Two transports, one dispatcher. Every command, from stdin or the socket, becomes the same `Command` value handled by today's `stdin_loop::handle`. The stdio path stays byte-for-byte as it is until the C++ engine is deleted. |
| S1 | Transport: a Unix domain socket on macOS and Linux; a named pipe on Windows later, behind `platform`. No TCP or WebSocket on localhost: any local process (and any browser page, for WebSocket) could reach it, and a UDS gets file-permission access control for free. Node connects with `net.createConnection(path)`. |
| S2 | Framing: newline-delimited JSON, one message per line, UTF-8. Same framing as today, readable with `socat`, trivial in Node, Rust, Python and shell. |
| S3 | Envelope: JSON-RPC 2.0. Requests have `id` and get one response; the engine's deltas are notifications. It is a known spec with clients in every language, so third-party tools and plugins don't need our library. |
| S4 | The protocol is multi-account from the first version. Every account-scoped request carries `accountId`; every delta notification carries `accountId`. Step 1 can then ship before step 2 without a protocol change. |
| S5 | Delta payloads keep today's shape (`modelClass`, `modelJSONs`, `type`) and today's coalescing and flush timing (`DeltaStream`). Only the envelope is new, so the desktop's delta handling and every plugin listening on `DatabaseStore` stay as they are. |
| S6 | Access control: the socket directory is `0700`, the socket `0600`, and the engine checks the peer uid (`getpeereid` / `SO_PEERCRED` in `platform.rs`) and drops other users. No tokens on the socket. |
| S7 | The CLI is Rust (`apps/cli`, binary `tern`) on a shared protocol crate (`crates/ms-ipc`). Rust over Go: it shares the protocol types and client with the engine, uses one toolchain, CI and `cargo deny`, and builds static binaries (musl on Linux). Go would duplicate every type. |
| S8 | Staging: socket transport inside today's per-account process first (phase A), CLI and read API next (phase B), single engine process after (phase C). Phase A is small and contained, and lets the desktop and harness move transport without also moving process model. |

## 1. Protocol (v1)

### 1.1 Connection

1. Client connects and sends `hello`. Nothing else is accepted before it.
2. The engine answers with its protocol version, build, and the accounts it holds.
3. The client calls `subscribe` if it wants deltas. A client that never subscribes (most CLI
   commands) gets only responses.

```jsonc
→ {"jsonrpc":"2.0","id":1,"method":"hello","params":{"protocol":1,"client":"tern-cli/0.1.0"}}
← {"jsonrpc":"2.0","id":1,"result":{"protocol":1,"engine":"0.1.0","pid":4242,
     "accounts":[{"accountId":"a1b2c3","email":"me@example.com","state":"running"}]}}
→ {"jsonrpc":"2.0","id":2,"method":"subscribe","params":{"accounts":null,"classes":null}}
← {"jsonrpc":"2.0","id":2,"result":{}}
← {"jsonrpc":"2.0","method":"delta","params":{"accountId":"a1b2c3","modelClass":"Thread",
     "type":"persist","modelJSONs":[…]}}
```

`protocol` is an integer. Additive changes (a new method, a new optional field) don't bump
it; a breaking change does, and the engine refuses a `hello` with a version it doesn't speak
(`-32000`, message names the versions it accepts).

### 1.2 Methods

Phase A (the stdin commands, renamed):

| Method | Params | Result | Today |
|---|---|---|---|
| `hello` | `protocol`, `client` | see 1.1 | — |
| `subscribe` | `accounts?`, `classes?` (null = all) | `{}` | stdout |
| `task.queue` | `accountId`, `task` | the task after its local phase | `queue-task` |
| `task.cancel` | `accountId`, `taskId` | `{}` | `cancel-task` |
| `sync.wake` | `accountId?` (none = all) | `{}` | `wake-workers` |
| `bodies.fetch` | `accountId`, `ids` | `{}` | `need-bodies` |
| `calendar.sync` | `accountId` | `{started: bool}` | `sync-calendar` |
| `engine.status` | — | accounts, `ProcessState`, uptime, versions | — |

`task.queue` replies after the local phase, which today's stdin path can't express. The
reply does not mean the task's deltas have been delivered (the main-thread store flushes
after 5 ms); a client that waits for completion watches `Task` deltas, as the desktop's
`TaskQueue` does.

`test-crash` and `test-segfault` stay stdin-only, debug builds only.

Phase B (read API, section 3): `folders.list`, `labels.list`, `threads.list`,
`threads.get`, `messages.get`, `search`, `contacts.search`, `tasks.get`. Every one takes
`accountId`.

| Method | Params | Result |
|---|---|---|
| `folders.list` | — | `folders` (Folder JSON by path), `counts` (`{id: {unread, total}}` from `ThreadCounts`) |
| `labels.list` | — | `labels`, `counts`, as above |
| `threads.list` | at most one of `folderId`, `labelId`, `role`; `limit` (50, max 500); `before` | `threads` (Thread JSON), `next` cursor or null |
| `threads.get` | `id` | `thread`, `messages` (oldest first, no bodies) |
| `messages.get` | `id`, `body?` (false) | `message`, `body` (null unless asked for and fetched), `files` |
| `search` | `query`, `limit` | `threads` matching in `ThreadSearch`, newest first |
| `contacts.search` | `query`, `limit` | `contacts` matching in `ContactSearch`, not hidden, most referenced first |
| `tasks.get` | `id` | `task` (the stored Task JSON); not found once cleanup dropped it (the newest 100 finished tasks per account are kept) |

`threads.list` orders by `(lastMessageTimestamp, id)` descending; `next` is opaque and
pages stay stable while threads are added. `role` picks the folder with that role, else the
label; an empty role is invalid. Like the desktop's mailbox perspective, a category other
than spam or trash lists only threads in all mail (`ThreadCategory.inAllMail`). Search text is plain: each whitespace-separated term is quoted for FTS5 and all must
match (contact terms as word prefixes), so no user text is read as FTS syntax.

Phase C (section 4, as built): `account.list` on both channels; `account.add`,
`account.remove` and `engine.shutdown` on `serve` mode's stdin only; notification
`account.state`. `account.test` and `account.reset` are not methods: `test` and `reset` stay
one-shot modes for now.

After phase A: `integrations.translate`,
`integrations.grammar`, `integrations.enrich`, `integrations.test`, served by the
integrations crate (provider HTTP, mapping, caching), which is written transport-free
before phase A lands. No stdin shim.

### 1.3 Errors

Standard JSON-RPC codes for parse (`-32700`, with `id: null`), invalid request, unknown
method and invalid params. Engine codes from `-32000` down: unsupported protocol, unknown
account, task rejected (with the `SyncError` JSON as `data`), hello required (`-32003`),
not found (`-32004`: a read named a folder, label, thread or message the account doesn't
have), account stopped (`-32005`, `data: {state, error}`), account exists (`-32006`,
`account.add` of a starting or running account), wrong channel (`-32007`: a parent-only
method on the socket, or `subscribe` on stdin). An unparseable line gets a
parse error instead of being ignored as stdin does.

### 1.4 Delivery and backpressure

- Coalescing stays in `DeltaStream`; its `Box<dyn Write>` becomes a `DeltaSink`. The stdio
  sink is today's stdout writer. The socket sink is a hub that fans each flushed line out to
  every subscriber whose filter matches.
- Each connection has a writer thread and a bounded queue (by bytes, generous for the
  desktop). A subscriber that falls behind past the bound is sent a final error and
  disconnected; the engine never blocks a flush on a slow client. The desktop reconnects
  and re-reads what it shows from the database, as it does after a relaunch.
- Responses and notifications share the connection's queue, in order.

### 1.5 Socket location and discovery

- Default socket directory `$CONFIG_DIR_PATH/run/`. Phase A: `run/<accountId>.sock`;
  phase C: `run/engine.sock`. `--socket <path>` overrides it, because macOS limits socket
  paths to 104 bytes and harness work dirs under `/var/folders` exceed it.
- After `bind` the engine writes `run/<name>.json` (`pid`, `socket`, `protocol`, `accountId`
  in phase A) and removes it on clean exit. The CLI finds engines by listing `run/*.json`
  and skipping entries whose pid is gone. A stale socket file is unlinked before `bind`
  only after a failed connect to it.

### 1.6 Threading

Blocking `std::os::unix::net`, matching the engine's thread model: one acceptor thread, one
reader and one writer thread per connection, commands sent to the dispatcher over a channel.
No tokio in the bin for this; the Windows named pipe can use tokio's `windows::named_pipe`
inside `platform` when it comes.

## 2. Phase A: socket transport, per-account process

Engine:
- `crates/ms-ipc`: envelope, method params and results, error codes, a blocking client
  (used by the CLI and the engine's tests). Depends on `serde`/`serde_json` only, like
  `ms-wire`.
- `apps/engine`: `--socket <path>` on `sync` mode. With it, deltas go to the socket hub
  instead of stdout, commands come from the socket, and stdin carries only the account and
  identity handshake (secrets stay off argv and off the socket) and the lifeline: closed
  stdin still exits 141 after 30 s. The engine prints `{"listening":"<path>"}` on stdout once
  bound, which the desktop waits for. `ProcessAccountSecretsUpdated` (the account JSON after
  an OAuth refresh) stays on stdout as a stdio delta line and never reaches the socket.
  SIGTERM and SIGINT remove the socket and discovery file before the engine dies of the
  signal. Without `--socket` nothing changes.
- `DeltaSink` in `ms-store`; `accountId` added to the notification envelope from the
  stream's account.
- Peer uid check and `run/` permissions in `platform.rs`.
- Tests: `apps/engine/tests/socket.rs` (hello, version refusal, subscribe filter, each
  method, slow-subscriber drop, peer check, stale socket).

Harness:
- `MAILSYNC_TRANSPORT=socket` in `test/harness/mailsync.py` (`start`, `send`,
  `_read_stdout`, about 70 lines): the same wrappers over `ms-ipc`'s wire format in Python.
  Quick tier runs in both transports; the phase gate runs the full suite on the socket.
  A/B against C++ stays on stdio.

Desktop (after the licence workflow's commits land):
- `mailsync-process.ts` `sync()` for the Rust engine: spawn with `--socket`, write the
  handshake on stdin as now, wait for `listening`, connect, `hello`, `subscribe`, and emit
  the same `deltas` events from notification params, plus the secrets deltas still read from
  stdout. `sendMessage` maps the four commands
  to methods. stderr and exit codes keep their meaning; socket errors map to today's
  close codes. C++ keeps the stdio path. The bridge is untouched.
- Hand-written TS types for the v1 messages next to `mailsync-process.ts`; generating them
  from `ms-ipc` (`packages/engine-protocol`) waits for step 3, when the surface grows.

Gate: engine tests, harness quick tier in both transports, full suite on the socket, a
desktop trial on `~/tern-trial` with the Rust engine (send, labels + undo, CalDAV).

## 3. Phase B: CLI, read API, binaries

CLI (`apps/cli`, `tern`, on `clap` + `ms-ipc`):
- `tern status`, `tern accounts`
- `tern watch [--account] [--class …]`: deltas, pretty or `--json` (NDJSON)
- `tern sync [--account]`, `tern calendar sync --account`
- `tern task queue <json|->`, `tern task cancel`, `tern task wait <id>`
- `tern folders`, `tern threads [--folder|--label] [--limit]`, `tern show <message-id>`,
  `tern search <query>`
- `tern logs [<account>] [-f]` (tails `mailsync-<id>.log`; no engine call; the account defaults
  to `--account`, else the only account with a log)
- `tern rpc <method> [params]`: raw call, for scripting and debugging
- `--config-dir` (default: the Tern profile dir per OS), `--json` on every command.

CLI output, as built. Exit codes: 0 ok, 1 the command failed (an engine error, a task that
failed, a timeout, a lost engine), 2 usage, 3 no engine running or none holds the account.
`--json` prints one document per command, or NDJSON for streams:
- `status`: `{engines: [engine.status result + socket]}`; `accounts`: `{accounts:
  [{accountId, email, state, pid, socket}]}`; `sync`: `{accounts: [id]}`; `calendar sync`:
  `{accountId, started}`; `rpc`: the method's result as is.
- `folders` / `labels`: `{folders|labels: [model], counts: {id: {unread, total}}}`, merged
  over the accounts.
- `threads`: `{threads: [Thread], next, more}`. `next` is the cursor for `--before` with one
  account (null on the last page, and always null for a page merged over several accounts);
  `more` says whether threads were left out either way. `search`: `{threads}`; `contacts`:
  `{contacts}`.
- `show`: `{kind: "message", message, body, files}` or `{kind: "thread", thread, messages}`.
- `task queue`: the task after its local phase; with `--wait`, and `task wait`: the final
  Task JSON (exit 1 if its `error` is set). `task cancel`: `{accountId, taskId}`.
- `watch`: NDJSON of `delta` params (`{accountId, modelClass, type, modelJSONs}`). When an
  engine's connection ends it prints `{event: "disconnect", pid, reason}` and exits 1, so a
  stream never silently loses an account.
- `logs`: NDJSON `{line}`.

`task wait` looks the task up with `tasks.get`, subscribes to `Task` deltas, then reads it
again, so a task that finished before the subscription is reported at once; an unknown id
fails. Without `--config-dir` or `TERN_CONFIG_DIR` the CLI uses the desktop's profile dir,
and on Linux falls back to the snap's `~/snap/mailspring/common` when that one has a live
engine. Output is cut to the terminal width only on a terminal (`COLUMNS` only when the
terminal doesn't report a size).

Read API (engine): the phase B methods in 1.2, on a read-only `Store` per connection,
returning the model JSON the desktop already parses, paged by `(lastMessageTimestamp, id)`.
It puts the queries in the engine rather than in the CLI, and it is the start of the query
API the thin desktop and plugins need in step 3 (`docs/plugins-inventory.md`, risky
couplings 1, 2 and 7).

Binaries: `dist` (cargo-dist, MIT/Apache) for GitHub releases: `tern` and the engine for
macOS arm64 and x86_64 and Linux x86_64 and aarch64 (musl, static), a shell installer and a
Homebrew formula. macOS builds signed and notarized with `apps/engine/packaging/macos`.
The CLI has no TLS or SQLite, so it stays small.

Release binaries, as set up (2026-10-16):
- Only `tern` ships. The standalone engine waits for the headless-engine decision (section
  4): it needs the engine to own credentials, and a release engine exits 2 unless argv[0]
  contains "tern" or "mailspring" (`apps/engine/src/main.rs`). The engine's own archives
  stay in `engine-release.yml`.
- dist 0.32.0, configured in `dist-workspace.toml`; `[profile.dist]` (release, thin LTO, no
  debug info) in the root `Cargo.toml`; `[package.metadata.dist] dist = true` in
  `apps/cli/Cargo.toml`, because `publish = false` hides a package from dist. `dist
  generate` writes `.github/workflows/cli-release.yml`, which is then hand-edited
  (`allow-dirty = ["ci"]`): top-level `contents: read`, `contents: write` only on `host`, and
  a path filter on `pull_request`. Re-apply those after regenerating. `precise-builds` keeps
  the build to `-p tern`.
- Targets: aarch64 and x86_64 macOS, x86_64 and aarch64 Linux musl, each on a native
  runner. Artifacts: `tern-<target>.tar.xz`, `tern-installer.sh` (installs to
  `~/.local/bin` or the XDG bin dir, no updater), `tern.rb`, `source.tar.gz`, checksums and
  `SOURCE.txt`.
- Tags are `cli/v<version>` (`cli/v0.1.0`), matching the `version` of `apps/cli`. dist only
  strips a tag prefix that names a package, so `cli-v0.1.0` fails to parse; any prefix
  before a `/` works. `tag-namespace = "cli"` keeps `engine-v*` tags out. PRs run `dist
  plan` only.
- Licences: every archive carries `LICENSE.md`, `NOTICE.md` and `THIRD-PARTY-NOTICES.txt`.
  `.github/cli-release-build-setup.yml` (dist's `github-build-setup`) installs cargo-about
  and runs `apps/engine/packaging/third-party-notices.sh` on `apps/cli/Cargo.toml` before
  `dist build`; `about.toml` lists the musl targets. `SOURCE.txt` comes from
  `.github/scripts/source-offer.sh`, as a dist extra artifact.
- Homebrew: the formula is attached to each release; the tap `ternmail/homebrew-tap` is
  named but not published to. Once the repo exists, add `publish-jobs = ["homebrew"]` and a
  `HOMEBREW_TAP_TOKEN` secret.
- TODO: macOS binaries are unsigned (the linker's ad hoc signature only). dist builds and
  archives in one step, so `apps/engine/packaging/macos` can't sign in between, and dist's
  own `macos-sign` signs without a secure timestamp or notarization. curl and Homebrew
  installs aren't quarantined, so this only bites a tarball downloaded in a browser.
- Checked locally: `dist plan`, `dist build` for aarch64-apple-darwin (archive holds `tern`
  and the three notice files), actionlint. The musl binaries were only built with `cargo
  build --profile dist` in a Docker container (both static); `dist build` for them, the
  x86_64 macOS build and the installers' global job first run on GitHub. The phase B gate
  ("dist build for all four targets in CI") needs a tag push, or `pr-run-mode = "upload"`
  for one PR (`SOURCE.txt` is then a placeholder, since only a tag push has a tag to name).

Gate: CLI integration tests against an engine on the fake server (the harness launches it),
`dist build` artifacts for all four targets in CI.

## 4. Phase C: one engine process (roadmap step 2)

`--mode serve`: starts with no account, binds `run/engine.sock`, and takes accounts over
`account.add {account, identity}`. The desktop spawns it once and keeps the stdin lifeline,
so its lifetime and crash reporting don't change.

What has to change (from the 2026-10-16 audit):
- **Logs.** The global subscriber writes to one file. Keep one `mailsync-<id>.log` per
  account (the desktop's crash reports and the harness's log parser read it) by routing each
  event on a thread-local account id set when each account's threads spawn; thread names
  stay as they are. Engine-wide lines go to `engine.log`.
- **Wake.** `ms_core::wake`'s global `WakeSignal` moves into `WorkerContext`;
  `sync.wake` without an account wakes all of them.
- **Failure isolation.** `abort_worker` on a non-retryable error, task-phase database errors
  and worker panics (caught with `catch_unwind` at each worker's top) stop that account's
  threads and send `account.state {accountId, state:"stopped", error}`, instead of aborting
  the process. The desktop's `CrashTracker` moves from process exits to these
  notifications. A panic outside account threads (dispatcher, socket) still aborts.
- **Per-account dispatch.** Each account gets the main thread's role (local phases,
  cancels, debounce) on its own thread fed by a channel, so one account's slow local phase
  doesn't hold up another's.
- **Tokens.** `TokenManager`'s single mutex is held across the HTTP refresh; make the lock
  per account.
- **Deltas.** One `DeltaStream` per account, each flushing into the shared hub with its
  account id. `ProcessIdentityRefreshNeeded` and the `none` class get `accountId` from the
  envelope.
- **Cache counters** (`LABELS_VERSION` / `FOLDERS_VERSION`) can stay global: a save in one
  account only makes the others reload their caches.
- **SQLite.** About five connections per account against one WAL file; watch busy waits
  with 8+ accounts.
- **One-shot modes.** `migrate` and `import-legacy` stay separate invocations before
  `serve` (the migration window keys on their stdout). `test` and `reset` become
  `account.test` and `account.reset`; the desktop's onboarding check and Reset Cache call
  them instead of spawning.
- **Desktop.** `mailsync-bridge.ts` `_clients` becomes one engine client; `ensureClients`
  diffs accounts into `account.add` / `account.remove`; `OnlineStatusStore` and the error
  states key on `account.state`; `attachToXcode` and the window-event-handler use the one
  pid.
- **Harness.** A per-account delta filter; `held()` (SIGSTOP) pauses every account, which
  is fine for its single-account scenarios; new multi-account scenarios (one account's
  fatal error leaves the other syncing, add and remove while running).

### 4.1 Decisions, as built (2026-10-16)

- **Modes.** `--mode serve` starts with no account and binds `--socket <path>`, by default
  `$CONFIG_DIR_PATH/run/engine.sock`. It writes `run/engine.json` `{pid, socket, protocol,
  accounts: [ids]}` and rewrites it on every add and remove. `migrate` and `import-legacy`
  stay one-shot invocations before `serve`; `test` and `reset` stay one-shot modes too (the
  desktop removes an account from the engine before a reset and adds it back after).
  `--mode sync`, on stdio or `--socket`, is unchanged: the C++ A/B path and the fallback.
- **Parent channel.** Secrets never go over the socket. `serve`'s stdin takes JSON-RPC
  requests in the socket's envelope: `account.add {account, identity}` (the account JSON
  and the identity line `sync` reads, parsed the same way, one `HttpMetadataBackend` per
  account), `account.remove {accountId}`, `engine.shutdown`, and every socket method
  (`hello` is not required, `subscribe` is refused). stdout carries their responses and the
  `ProcessAccountSecretsUpdated` deltas as `delta` notifications with `accountId`, and
  nothing else. stdin is read once the socket is bound, so the first response marks the
  engine ready. The socket refuses `account.add`, `account.remove` and `engine.shutdown`
  with `-32007`. Closed stdin still exits 141 after 30 s; SIGTERM / SIGINT remove the socket
  files as in phase A. When another engine answers on the socket (the previous one still
  shutting down after a window reload), serve exits 3 with the one-shot error; the desktop
  spawns it again every second, up to 30 times, without counting a crash.
- **States.** `account.state {accountId, state, error?}`: `starting` (added),
  `running` (its `main` thread opened the database), `stopped` (with the `SyncError` JSON),
  `removed`. The state and error are also in `hello`, `engine.status` and `account.list`.
  `account.state` goes to every subscriber of that account whatever its class filter, and
  is held in the first subscriber's backlog like deltas. `account.add` answers once the
  account is `running` (or `stopped`, or after 10 s with `starting`); adding a starting or
  running account is `-32006`, adding a stopped one restarts it (after its threads have
  been joined). `account.remove` stops the runtime, answers `{stillRunning: [thread
  names]}` (threads left past the 5 s stop timeout, which may still write to the database;
  the desktop waits and retries before a reset), sends `removed` and drops the account from
  the lists; its database rows stay. A stopped or removed account's `DeltaStream` is closed,
  so a thread left running cannot publish under its id any more.
- **Failure isolation.** A non-retryable worker error, a task local-phase database error,
  `test.crash`, or a panic on one of an account's threads stops only that account: its
  threads return cooperatively (IDLE interrupted, sleeps woken), an `accountStop` thread
  joins them (up to 5 s, then leaves stragglers) and then sends `stopped` with the error.
  The engine never restarts an account. In `serve` the panic hook logs a panic on an
  account thread at error and lets `WorkerContext::spawn` catch it; a panic on an
  engine-wide thread (stdin, socket, signals, `accountStop`) still aborts. Per-account
  failures write nothing to stderr. Commands for a stopped account get `-32005`; reads
  still answer.
- **Logs.** One `mailsync-<accountId>.log` per account, same format, rotation and thread
  names, routed by the account id each of its threads carries (short-lived ones included).
  Engine-wide lines (socket, stdin, hub) go to `$CONFIG_DIR_PATH/engine.log`. `sync` mode
  opens no account file, so every line lands in its one log as before.
- **Per-account machinery.** `WakeSignal` in `WorkerContext`; `sync.wake` without
  `accountId` wakes every running account. One `TokenManager` per account (so no lock is
  shared across accounts at all). One `DeltaStream` per account with its own flush thread
  into the shared hub, coalescing and timing unchanged. Each account's local phases,
  cancels and debounce run on its own `main` thread fed by a channel. The read API and
  every account-scoped method route by `accountId`; unknown is `-32001`.
- **Debug only.** `test.crash {accountId}` and `test.panic {accountId}` on stdin, the
  per-account `test-crash`.

Out of scope for phase C, and the next real decision: a headless engine started by the CLI
without the desktop. It needs the engine to own account settings and credentials (OS
keychain), since it can't read the desktop's safeStorage keyset. The same move would let
integrations read their API keys in the engine instead of receiving them per request.

## 5. Order and size

| Phase | Packages (parallel within a phase) | Rough size |
|---|---|---|
| A | `ms-ipc` + engine socket (serial first), then harness socket mode, desktop transport | ~2k lines Rust, ~150 Python, ~300 TS |
| B | read API, CLI, `dist` + CI | ~2.5k Rust, CI config |
| C | logs, failure isolation, per-account dispatch + wake + tokens, `serve` + account methods, desktop bridge, harness multi-account | ~3k Rust, ~600 TS |

Each phase is one workflow, gated as above, committed per package.

## Status

2026-10-16, phase A implemented and gated, not yet committed or trialled in the desktop.

Delivered:
- `crates/ms-ipc`: envelope, the eight v1 methods' params and results, error codes,
  `delta` and `disconnect` notifications, NDJSON framing (64 MiB line limit), a blocking
  `Client`.
- Engine: `--socket <path>` on `sync`. stdin is read on its own thread and both transports
  feed one dispatcher (`apps/engine/src/dispatch.rs`, S0). stdin still takes the old
  commands as well as the handshake and lifeline. `ProcessAccountSecretsUpdated` stays on
  stdout. Deltas flushed before the first unfiltered subscriber (up to 16 MiB) are held and
  replayed to it, so early `ProcessState` isn't lost. SIGTERM/SIGINT and exit 141 remove the
  socket and discovery file. On Windows `--socket` fails with a one-shot error.
- Harness: `MAILSYNC_TRANSPORT=socket` (`test/harness/mailsync.py`). Scenarios pinning a
  `binary:` run it on stdio.
- Desktop: `sync()` with the Rust engine uses the socket by default
  (`app/src/engine-protocol.ts`, `mailsync-process.ts`), at `run/<accountId>.sock`, or
  `$TMPDIR/tern-<uid>/<hash>.sock` when that path is too long. The bridge is unchanged.

Selecting a transport: the engine uses stdio unless given `--socket`. The harness uses stdio
unless `MAILSYNC_TRANSPORT=socket`. The desktop uses the socket for the Rust engine on macOS
and Linux; `TERN_ENGINE_TRANSPORT=stdio` switches back, and the C++ engine, Windows and the
one-shot modes always use stdio.

Gate results: fmt, clippy `-D warnings` and `cargo deny` clean. `cargo test --workspace`:
620 passed, 43 ignored. `tests/socket.rs` passed 5 runs in a row; the ignored exit-141
tests pass. Harness quick tier: 117 passed, 1 skipped, on both stdio and socket. Full suite
on socket: 227 passed, 3 skipped. Every skip is `rules-ready-pre-upgrade-messages`, whose
baseline binary isn't built. Desktop typecheck and eslint are clean. The spec ran 26/26
under a plain-Node jasmine stand-in, and once end-to-end against the debug engine.

Open:
- Desktop trial on `~/tern-trial` with the Rust engine (send, labels + undo, CalDAV), to be
  done by the coordinator.
- Windows named pipe: not started. The non-Unix cfg paths haven't been checked against
  `x86_64-pc-windows-msvc`.
- The desktop spec still has to pass under the real Electron runner after rebasing onto main
  (the worktree's spec runner imports `moment-round`, which main dropped).
- The handshake isn't moved: the harness still passes the account on `--account`, and stdin
  still takes commands in socket mode. The default `--socket` path from 1.5 isn't wired;
  the flag always takes a value.
- The peer uid refusal has no test, because it needs a second OS user. There's no cap on
  concurrent connections (each one costs two threads).
- An engine nobody subscribes to holds up to 16 MiB of deltas. Decide whether to keep that.
- The desktop reports a refused `hello` or a dropped socket as close -2, after a 1 s wait
  for the real exit. The user isn't told about a protocol mismatch any other way.
- `--orphan --socket` still logs to stdout.

Phase B read API, 2026-10-16, implemented, not committed:
- `crates/ms-ipc/src/read.rs`: params and results of the seven read methods (1.2), the
  `NOT_FOUND` code, `READ_DEFAULT_LIMIT` / `READ_MAX_LIMIT`.
- `crates/ms-store/src/read.rs`: `ReadStore`, a read-only connection (same busy timeout,
  `query_only`), with the account-scoped queries and the FTS5 quoting (`fts_query`).
- `apps/engine/src/read_api.rs`: answers the read methods on the socket connection's reader
  thread, on a `ReadStore` opened at the connection's first read. They never reach the
  dispatcher. `apps/engine/tests/read_api.rs` checks each method on a seeded database,
  including paging across inserts, filters, bodies, FTS syntax in user text, not-found, and
  a read answering while a task's local phase waits on the write lock.
- `tasks.get` (the stored Task by id), for `tern task wait`; `tests/read_api.rs`
  `tasks_get`.

Open (read API):
- Requests on one connection are answered in order, so a read queued behind a slow
  `task.queue` on the same connection waits for it. Clients that need both use two
  connections.
- `Thread.lastMessageTimestamp` has no index; an account-wide `threads.list` sorts in
  memory. The desktop's lists use `lastMessageReceivedTimestamp` (indexed through
  `ThreadCategory`). Add an index in a migration if big mailboxes are slow.
- `search` is plain terms only: no `from:`/`subject:` operators or prefix matching, and no
  ranking beyond recency. `messages.get` returns `files` from the message JSON, not the
  `File` table.
- `threads.list` with no folder, label or role lists every thread of the account, spam and
  trash included. A stored row whose JSON doesn't parse comes back as null.

Phase B, 2026-10-16, implemented and gated, not committed:
- Read API: above, plus `tasks.get`.
- `apps/cli` (`tern`): the commands, `--json` shapes and exit codes in section 3. It finds
  engines through `<config-dir>/run/*.json`, skips dead pids, says `hello` to each, and
  without `--account` merges read results over every account. Dependencies: clap, ms-ipc's
  client, serde, chrono, libc (Unix); no tokio, TLS or SQLite (1.8 MB release binary).
- Release config: `dist-workspace.toml`, `.github/workflows/cli-release.yml`,
  `.github/cli-release-build-setup.yml`, `[profile.dist]` (section 3 "Release binaries").
- CI: `engine.yml` builds `-p mailsync -p tern` and runs `test_cli.py` with
  `TERN_BIN`; the module fails instead of skipping when `CI` or `REQUIRE_TERN` is set.

Install and run, until the first `cli/v*` release:

```sh
cargo build --release -p tern            # or: cargo install --path apps/cli --locked
target/release/tern status               # engines the desktop started, via its profile dir
tern threads --folder inbox -n 10
tern show <thread-or-message-id> --body
tern watch --json | jq .
tern task queue '{"__cls":"ChangeStarredTask","messageIds":["…"],"starred":true}' --wait
TERN_CONFIG_DIR=<harness run>/config tern status   # an engine the harness started
```

The engine has to run with `--socket` (the desktop's default for the Rust engine, or the
harness with `MAILSYNC_TRANSPORT=socket`). After a release: the shell installer
`tern-installer.sh`, or `tern.rb` once the tap is published.

Gate results: fmt, clippy `-D warnings` (`--locked`) and `cargo deny` clean. `cargo test
--workspace --locked`: 709 passed, 43 ignored, 73 binaries. Harness quick tier, release
build, with `REQUIRE_TERN=1`: 118 passed, 1 skipped on stdio and the same on socket (the
skip is `rules-ready-pre-upgrade-messages`, baseline binary not built); that includes
`test_cli.py`. `dist plan` lists the four targets, each archive with `tern`, `LICENSE.md`,
`NOTICE.md` and `THIRD-PARTY-NOTICES.txt`; `dist generate --check` passes. `dist build
--artifacts=local --target=aarch64-apple-darwin --tag=cli/v0.1.0` built an archive with those
four files; `tern --version` runs from it. Tried by hand against one harness engine on
`fake:dovecot`: status, accounts, folders, threads, search, show and `show --body` on a
message older than the prefetch window, `task queue --wait` while `watch` printed its Task
deltas, sync, logs, and the not-found and no-engine exits.

Open (phase B):
- The phase B gate's `dist build` for all four targets in CI hasn't run: it needs a
  `cli/v*` tag push or one PR with `pr-run-mode = "upload"`. The musl builds, x86_64 macOS
  and the global job (installer, formula, `SOURCE.txt`) have only been planned. The plan and
  announce jobs now run with a read-only token; unverified until that first run.
- macOS signing and notarization (TODO above). Homebrew publishing waits for the tap repo.
- The hand-edited `cli-release.yml` and `engine.yml` haven't been through actionlint.
- Merged multi-account `threads` pages have no cursor; paging needs `--account`.
- Display width counts chars, so CJK and emoji can overrun a truncated row.
- `show --body` strips HTML tags roughly. `show <thread-id>` has an empty snippet column for
  messages whose body isn't fetched.
- The Linux snap fallback (`~/snap/mailspring/common`) is untested on a snap install.
- The e2e test sleeps 1 s after starting `watch`, since nothing it prints marks the
  subscription.
- `apps/engine/CLAUDE.md`'s layout table and `apps/engine/test/README.md` don't mention
  `apps/cli` and `test_cli.py` yet.

Phase C, 2026-10-16, implemented and gated, not committed or trialled in the desktop.

Delivered:
- C1, isolation: `ms_sync::runtime` (`AccountRuntime`, `WorkerContext::fail` / `spawn` /
  `request_stop`), per-account log routing in `ms_core::log` (`mailsync-<id>.log` per
  account, `engine.log` for the rest), `WakeSignal` / `DeadlineSignal` in the account's
  context and closable, per-account `TokenManager` locks. Workers check the stop flag at
  their loop points and every sleep wakes on stop. `sync` mode's failure handler logs
  critical and aborts with the same text as before.
- C2, `serve`: `apps/engine/src/serve.rs` (registry, parent channel, `accountStop`
  reaper), `socket.rs` over an `Accounts` trait (one account in `sync`, the registry in
  `serve`) with a multi-account hub and a `Session` both channels use. `ms-ipc`:
  `account.*`, `engine.shutdown`, `test.*`, `account.state`, their params and results
  (`AccountRemoveResult {stillRunning}` included), `error` on `AccountInfo` /
  `AccountStatus`, codes `-32005` to `-32007`. Protocol stays 1. `EXIT_SOCKET_IN_USE` (3)
  when another engine answers on the socket. The metadata stream ends on stop
  (`MetadataBackend::pull` takes a stop check); `DeltaStream::close` drops what a stopped
  account's leftover threads emit.
- CLI: discovery reads `run/engine.json` next to `run/<id>.json`; `tern status` and
  `tern accounts` show each account's state and error key; `tern sync` without
  `--account` skips stopped accounts.
- Harness: `MAILSYNC_TRANSPORT=serve` (one engine per scenario, accounts added on stdin,
  deltas routed by `accountId`, checks that stdout carries only responses and secrets and
  that no delta crosses accounts). Multi-account scenarios (`account:` on any step,
  `account.remove` / `account.add`, `wait {stopped}`, `server.set_password`; expectations
  `accounts`, `stopped`, `engine_log`): `multi-account-side-by-side`,
  `-fatal-error-isolated`, `-remove-add`, `-task-routing`, on all three transports.
  `test_cli.py` runs against socket and serve.
- Desktop: `EngineServeProcess` (`mailsync-process.ts`) and `EngineClient`
  (`app/src/engine-client.ts`, with `CrashTracker` moved there). `ensure()` diffs
  AccountStore into `account.add` / `account.remove`; commands route by `accountId` and
  are held until the add answers; `account.state` stopped counts as a crash for that
  account and re-adds it (5 in 5 min, then the error state); an engine exit respawns and
  re-adds every account, with backoff (0, 1, 2, 4, 8 s) and its own 5-in-5-min cutoff;
  exit 3 respawns every 1 s up to 30 times without counting a crash. Reset Cache removes
  the account, waits up to 10 s when `stillRunning` isn't empty, runs the one-shot
  `reset`, then adds it back. Secrets the client added are redacted from engine errors.
  Spec: `app/spec/engine-client-spec.ts`.

Selecting a process model:
- Engine: `--mode serve` runs every account in one process; `--mode sync` (stdio, or
  `--socket`) is one account per process, unchanged, and is what the C++ engine offers.
- Desktop: the Rust engine runs one `serve` engine by default on macOS and Linux.
  `TERN_ENGINE_PROCESS=per-account` goes back to phase A's per-account socket processes,
  and `TERN_ENGINE_TRANSPORT=stdio` to per-account stdio. The C++ engine, Windows and the
  one-shot modes (`migrate`, `import-legacy`, `test`, `reset`) are unchanged.
- Harness: `MAILSYNC_TRANSPORT=stdio` (default), `socket` or `serve`.

Gate results: fmt, clippy `-D warnings --locked` and `cargo deny` clean. `cargo test
--workspace --locked`: 724 passed, 0 failed, 44 ignored; the ignored exit-141 tests (sync
stdio, sync socket, serve) pass. Harness quick tier, release build, `REQUIRE_TERN=1`: 123
passed, 1 skipped on each of stdio, socket and serve. Full suite on serve with Docker:
233 passed, 3 skipped. Every skip is `rules-ready-pre-upgrade-messages` (baseline binary not
built). Desktop: tsc, eslint and prettier clean; under the Electron spec runner (with the
worktree's debug engine at `app/mailsync-rs`) `engine-client-spec` 20/20 and the full
suite 2764 passing, 0 failing. `--mode sync` stdout reviewed against the diff: deltas, the
`listening` line and the one-shot output are unchanged; the abort texts on stderr are the
same.

Open (phase C):
- Desktop trial of `serve` mode on `~/tern-trial` (two real accounts: send, labels + undo,
  CalDAV, Reset Cache, account add/remove, quit and relaunch), to be done by the
  coordinator.
- Next: wire `integrations.*` on the serve engine.
- Quit: the harness SIGTERMs on restart (as a client quitting) because `engine.shutdown`
  lets an in-flight `SendDraftTask` finish, which changed `sent-copy-send-interrupted`'s
  outcome. The desktop sends `engine.shutdown` on unload and kills after 8 s. Decide
  whether quit should shut down or kill.
- A thread blocked in a network call sees the stop only when the call returns. `stop`
  waits 5 s, then leaves it running; it may still write that account's rows (its deltas
  are dropped). The desktop waits a fixed 10 s on `stillRunning` before a reset; the engine
  can't report when such a thread ends. Re-adding the account starts new threads beside it.
- After a stop the metadata reader keeps its HTTP stream open until the next heartbeat (up
  to 5 s).
- serve's stdin handles requests one at a time: a slow `task.queue` or an `account.add`
  (up to 10 s each) delays a later `account.add` or `engine.shutdown`.
- A command in flight when its account stops gets `-32603` "the engine is shutting down",
  not `-32005`.
- Each account has its own `TokenManager`, so `TokenManager::forget` is unused.
- `test.crash` / `test.panic` are debug-only in serve, while sync's stdin `test-crash`
  works in release builds; the harness's fatal-error scenario uses an auth failure.
- `tern watch` ignores `account.state`. No desktop UI uses `starting` / `running` yet.
- No test drives a non-retryable error from inside a real worker in `serve` (they share
  `ctx.fail_with` with `test.crash`); no serve test with a Tern identity.
- In `sync` socket mode a subscriber filtered to its own account no longer takes the
  backlog (only `accounts: null` and no class filter does). The desktop and harness
  subscribe with `accounts: null`.
- On stdio and socket the stopped account's process aborts with SIGABRT, so macOS may
  write a crash report for it in the multi-account scenarios.
- Multi-account scenarios use the fake server only; `durations.json` isn't refreshed for
  them.
- `--mode sync` still exits 1 on a socket collision.
- Desktop specs need an engine binary at `apps/desktop/app/mailsync-rs` in the worktree, or
  the runner prints nothing.
