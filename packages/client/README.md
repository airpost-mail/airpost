# @tern/client

The TypeScript side of the engine's socket protocol, generated from `crates/tern-ipc`: every
method's params and result (`RpcMethods`), the notifications (`RpcNotifications`), the model
and task JSON (`ThreadJson`, `MessageJson`, `ChangeUnreadTaskJson`, ...,
`ModelJsonByClass`), and the constants with their doc comments (`Method`, `Feature`,
`ErrorCode`, `SETTINGS_KEYS`, `METHOD_CLASSES`, ...). The desktop
(`apps/desktop/src/engine/protocol.ts`) and the web (`apps/web/src/engine/rpc.ts`,
`types.ts`) re-export it.

Nothing here is written by hand except `scripts/generate.mjs`:

- `cargo run -p tern-ipc --bin tern-ipc-schema` writes `schema/tern-ipc.json`, one JSON
  Schema (draft 2020-12) document with the Rust types' doc comments, and
  `crates/tern-store/src/model_query_allowlist.json`.
- `node scripts/generate.mjs` turns the schema into `src/protocol/types.ts`
  (json-schema-to-typescript), `constants.ts` and `index.ts`.

`npm run generate` runs both, `npm run check` checks that the checked-in files are current
(CI runs it, and tern-ipc's tests fail on a stale schema). After changing a tern-ipc type, run
`npm run generate` here and commit the outputs with the change.

Types read as the engine writes them: a field it leaves out when unset is `?: T`, one it
writes as `null` is `?: T | null`, and a method's params accept `null` where the engine reads
it as absent.
