# @airpost/client

The shared TypeScript client for the engine's socket protocol, used by the web
(`apps/web/src/engine/ws.ts`, `settings.ts`) and the desktop windows
(`apps/desktop/src/engine/client/connection.ts`). Framework-free: no React, no RxJS, and no
Node outside the `@airpost/client/node` entry (docs/thin-clients-plan.md section 3.3).

| Module | What it is |
|---|---|
| `src/protocol/` | Generated from `crates/airpost-ipc` (below) |
| `src/rpc.ts` | JSON-RPC 2.0: the envelope, `RpcError` (the engine answered with an error), `EngineUnavailableError` (no answer: no connection, or it closed; `code` null), the `Transport` interface and `RpcPeer` (ids, pending calls, timeouts, answers in any order, requests from the engine) |
| `src/session.ts` | `Session`: hello, subscribe (with `since` from the `since` option, and `onSubscribed` for its answer), features, capabilities, connection state, reconnect with backoff, calls held until open, `resubscribe`, handlers for engine requests. Each client says in its options where to connect and what an ended connection means |
| `src/transports/ws.ts` | `webSocketTransport`, one message per text frame |
| `src/transports/unix.ts` | `unixTransport`, NDJSON on a Unix socket with Node `net`; exported only by `@airpost/client/node` (`node/package.json`, `src/node.ts`) |
| `src/settings.ts` | `SettingsCache`: engine settings read synchronously, filled by `settings.get`, kept current by `settings.changed`, writes shown at once and reverted when refused |
| `src/live.ts` | `LiveQueries` and `LiveQuery`: live queries over `query.subscribe`, `query.setWindow` and `query.changed`, with the window's rows in the engine's order, `total`, steps applied in `version` order (changes that arrive before their answer are held), window moves, a new query under the same `key`, and one `query.subscribe` per query after a reconnect; `watch()` shares a subscription between callers. `FeedCursor` keeps the raw feed's last `delta.seq` for `subscribe {since}`. Any client with `request`, notifications and hello/disconnect events can be a `LiveSource`; a `Session` is one |
| `src/tasks.ts` | `waitForTask`: a task's end from `task.done` (the engine sends it for `task.queue {notify: true}`) or from `Task` deltas from an engine without it; `taskDoneIn` reads the notification, with the finished task |

`npm run typecheck` checks the browser-safe part with no DOM or Node types
(`tsconfig.json`, `types/globals.d.ts` declares the timers and `console`) and the node entry
with Node's (`tsconfig.node.json`). `npm test` runs the specs in `test/` (vitest).

## Generated protocol

`src/protocol` holds every method's params and result (`RpcMethods`), the notifications
(`RpcNotifications`), the model and task JSON (`ThreadJson`, `MessageJson`,
`ChangeUnreadTaskJson`, ..., `ModelJsonByClass`), and the constants with their doc comments
(`Method`, `Feature`, `ErrorCode`, `SETTINGS_KEYS`, `METHOD_CLASSES`, ...). The desktop
(`apps/desktop/src/engine/protocol.ts`) and the web (`apps/web/src/engine/rpc.ts`,
`types.ts`) re-export it. Nothing there is written by hand except `scripts/generate.mjs`:

- `cargo run -p airpost-ipc --bin airpost-ipc-schema` writes `schema/airpost-ipc.json`, one JSON
  Schema (draft 2020-12) document with the Rust types' doc comments, and
  `crates/airpost-store/src/model_query_allowlist.json`.
- `node scripts/generate.mjs` turns the schema into `src/protocol/types.ts`
  (json-schema-to-typescript), `constants.ts` and `index.ts`.

`npm run generate` runs both, `npm run check` checks that the checked-in files are current
(CI runs it, and airpost-ipc's tests fail on a stale schema). After changing a airpost-ipc type, run
`npm run generate` here and commit the outputs with the change.

Types read as the engine writes them: a field it leaves out when unset is `?: T`, one it
writes as `null` is `?: T | null`, and a method's params accept `null` where the engine reads
it as absent.
