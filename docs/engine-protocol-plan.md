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
`messages.get`, `search`, `contacts.search`.

Phase C (section 4): `account.add`, `account.remove`, `account.test`, `account.reset`,
`engine.shutdown`; notifications `account.state` (running, stopped with error).

After phase A: `integrations.translate`,
`integrations.grammar`, `integrations.enrich`, `integrations.test`, served by the
integrations crate (provider HTTP, mapping, caching), which is written transport-free
before phase A lands. No stdin shim.

### 1.3 Errors

Standard JSON-RPC codes for parse (`-32700`, with `id: null`), invalid request, unknown
method and invalid params. Engine codes from `-32000` down: unsupported protocol, unknown
account, task rejected (with the `SyncError` JSON as `data`). An unparseable line gets a
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
  bound, which the desktop waits for. Without `--socket` nothing changes.
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
  the same `deltas` events from notification params. `sendMessage` maps the four commands
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
- `tern logs <account> [-f]` (tails `mailsync-<id>.log`; no engine call)
- `tern rpc <method> [params]`: raw call, for scripting and debugging
- `--config-dir` (default: the Tern profile dir per OS), `--json` on every command.

Read API (engine): the phase B methods in 1.2, on a read-only `Store` per connection,
returning the model JSON the desktop already parses, paged by `(lastMessageTimestamp, id)`.
It puts the queries in the engine rather than in the CLI, and it is the start of the query
API the thin desktop and plugins need in step 3 (`docs/plugins-inventory.md`, risky
couplings 1, 2 and 7).

Binaries: `dist` (cargo-dist, MIT/Apache) for GitHub releases: `tern` and the engine for
macOS arm64 and x86_64 and Linux x86_64 and aarch64 (musl, static), a shell installer and a
Homebrew formula. macOS builds signed and notarized with `apps/engine/packaging/macos`.
The CLI has no TLS or SQLite, so it stays small.

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
