# CLAUDE.md

The Rust port of Mailspring-Sync: the `airpost-engine` bin here, the `airpost-*` libraries in
`../../crates/`. Upstream Mailspring-Sync (https://github.com/Foundry376/Mailspring-Sync) is
the reference for behaviour; C++ paths in comments (`MailSync/…`, `Vendor/…`) are relative to
it, and its `CLAUDE.md` documents the invariants (message identity, placements, the orphan
sweep, undo pairing) that the port keeps. A/B runs use the C++ engine Mailspring.app ships
(`MAILSPRING_ENGINE_BIN` overrides the path).

## Layout

| Crate | Owns |
|---|---|
| `airpost-core` | errors (`SyncError`, `MailError`, `Error`, `Result`), ids, base58, constants, `Clock`, the log sink, `logmsg`, `Account`/`Identity`, header and flag types, `UidSet`, modified UTF-7, RFC 2047, dates, wake signals, protocol loggers |
| `airpost-store` | schema, `migrate`/`reset`, `Store`, `Model`, `Query`, models, placements, `DeltaStream`, the read side (`ReadStore`, `query`, Thread `summary`, the fields `derived.rs` adds to what clients read) |
| `airpost-providers` | `ServerProfile`, `resolve`, folder roles, OAuth endpoints |
| `airpost-imap` | the `ImapSession` trait and its async-imap `Session` |
| `airpost-mime` | parse and render bodies, snippets, the outgoing builder |
| `airpost-smtp` | `SmtpClient` on lettre |
| `airpost-net` | HTTP, OAuth tokens, `MetadataBackend` / `LocalOnly`, People API |
| `airpost-dav` | DAV XML, vCard, ICS fields, discovery |
| `airpost-sync` | worker threads, `MailProcessor`, `MailboxModel`, `WorkerContext`, the task runner seam |
| `airpost-tasks` | `TaskProcessor` |
| `airpost-ipc` | the socket protocol: JSON-RPC envelope, method types, model and task view structs, the `query` allowlist (`query_attrs.rs`), the JSON Schema that `packages/client` generates its TS from (`airpost-ipc-schema`), NDJSON framing, blocking client |
| `airpost-engine` (here) | CLI, modes, stdin reader, main-thread dispatch, `--socket` server, `serve` mode (`serve.rs`: account registry, parent channel), thread startup |

Dependency order: `airpost-core` ← everything; `airpost-store`, `airpost-providers`, `airpost-mime`, `airpost-net`
on `airpost-core`; `airpost-imap`, `airpost-smtp` also on `airpost-providers`; `airpost-dav` on `airpost-net`; `airpost-sync`
on all of those; `airpost-tasks` on `airpost-sync`. Workers reach `TaskProcessor` through
`airpost_sync::TaskRunnerFactory`, which the bin fills with `airpost_tasks::Tasks`.

## Commands

```sh
# from the repo root
cargo build --release                          # target/release/airpost-engine
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p airpost-engine -- --ignored            # the 30 s stdin-closed exit test
cargo deny check
python3 apps/engine/tools/schema_check.py check <airpost.db>  # compare against V12 plus migrations
AIRPOST_ENGINE_BIN=target/release/airpost-engine python3 -m pytest apps/engine/test -n 0 -k <scenario>
python3 -m pytest apps/engine/test             # quick tier: fake server, no Docker, ~3 min
python3 -m pytest apps/engine/test --all       # both tiers, every server kind, ~12 min
```

Which harness tier when (apps/engine/test/README.md "Tiers"): while iterating, a narrow
`-k` selection, then the quick tier before calling a change done. Run `--all` (or
`--servers <kind>` for the kind you touched) once per phase or step that changes IMAP,
DAV or Docker-server behaviour, and before a commit that ends a phase. A new fake-server
scenario that takes over ~20 s on its own gets `slow: true`.

Release builds exit 2 unless the lowercased argv[0] contains `airpost`; the harness symlinks the
binary as `<work>/airpost-mailspring-bin/engine` (the name also satisfies the C++ engine's
`mailspring` check, for A/B runs). Debug builds skip the check.

## Working in a shared tree

Several agents may work in this tree at once.

- The shared interfaces (`ImapSession`, `Model`, `Store`'s public methods, `MailboxModel`,
  `MetadataBackend`, `RemoteTaskRunner`/`TaskRunnerFactory`, `WorkerContext`) are used across
  crates. Adding a method or field is fine; changing or removing one breaks other crates, so
  make the smallest change and say so in your hand-off.
- `airpost-core` is shared by everyone. Add to it (a `logmsg` function, a constant); don't
  change what is there without checking every user.
- Add dependencies to `[workspace.dependencies]` in the root `Cargo.toml` and reference them
  with `.workspace = true`. Licences must pass `cargo deny check` (MIT, Apache-2.0, ISC,
  BSD, MPL-2.0, Zlib, Unicode-3.0). Never enable a feature that pulls `aws-lc-rs`
  or OpenSSL: TLS is rustls with ring.
- Don't commit, rebase or otherwise change git state unless asked; don't reformat files
  you are not changing beyond what `cargo fmt` does.
- Leave the workspace building, formatted and clippy-clean when you stop.

## Comment style

Write for a technical reader who prefers self-documenting code. Prefer clearer names and
smaller functions over commentary; a comment that restates the code should be deleted.

- State current behaviour and rationale: why the code is the way it is, and what a reader
  would otherwise get wrong. Never narrate thought process or the path to the answer.
- Never describe history (where code used to live, what the previous version did, what a
  diff changed). Git records that. Naming the C++ function a Rust item ports is fine and
  useful: it is where the reader finds the reference behaviour.
- Cite external evidence when it exists: a Sentry issue id, an upstream commit or PR, a
  spec section (RFC 9738 §3), a provider quirk, a harness scenario name.
- Item docs (`///`, `//!`) carry contracts; inline `//` comments sit on one non-obvious
  line and run one or two lines. `// Note:` marks a short inline remark. Longer inline blocks
  only for protocol or provider quirks, with the evidence cited.
- Every Rust file starts with `// SPDX-License-Identifier: GPL-3.0-only` and
  `// Copyright (C) 2026 Airpost contributors`. A file that ports C++ code then opens with a
  module doc naming the C++ files it derives from and the upstream copyright (Foundry 376,
  GPL-3.0), as `airpost-core/src/lib.rs` does. Code ported from mailcore2, libetpan or
  iCalendarLib also names that holder and licence and its NOTICE.md section, as
  `airpost-imap/src/lib.rs` does.

## Logging

- Use `tracing` (`info!`, `warn!`, `error!`) and `airpost_core::critical!` for what ends the
  process. Levels print as `info`, `warning`, `error`, `critical`; nothing below INFO is
  recorded, and only events from `airpost_*` targets reach the log.
- Top-level steps are unprefixed (`Sync loop complete.`); sub-steps use `"- "` then
  `"-- "`; failures use `"-X "`. Present tense, one line, no trailing newline.
- Any line the harness or the client matches on is a function in `airpost_core::logmsg`, with
  the scenario that depends on it in its doc. Never inline such a string, and change its
  wording only together with the scenario. `airpost-core/tests/logmsg.rs` checks every function
  against the scenario regexes; add a case when you add a function.
- Threads that log must be spawned with `std::thread::Builder::name`, using the names in
  `airpost_core::constants::thread` (`background`, `foreground`, `calContacts`, `metadata`,
  `metadataExpiration`, `calendar`). The log line carries the thread name and the harness
  reads it. An account's threads, short-lived ones included, are spawned with
  `WorkerContext::spawn`, which also carries the account id that routes their lines to
  `engine-<accountId>.log` when one process holds several accounts (`airpost_core::log`).
- Protocol traffic goes through `airpost_core::protolog::ProtocolLogger` (`sent …` / `recv …`, one
  line per protocol line), and AUTH payloads are `Direction::SentPrivate` so they are never
  logged.
- stderr stays empty: the client treats any stderr output as a crash. Never use `eprintln!`
  or `println!`; stdout writes go through `output.rs` in the bin (or the `DeltaStream`)
  and ignore errors, because a closed pipe must not panic.

## Coding conventions

- **Naming.** snake_case ports of the C++ names: `perform_local_x` / `perform_remote_x`,
  `sync_*` / `fetch_*`, `find` / `find_all`. JSON keys and SQL identifiers stay exactly as the
  C++ wrote them (`_sa`, `hMsgId`, `aid`, `v`, camelCase columns). Tunables live in
  `airpost_core::constants` with the C++ name in their doc.
- **Errors.** Library functions return `airpost_core::Result<T>`. Protocol crates return
  `Result<T, MailError>`; convert with `MailError::into_sync("<call>")` so the debuginfo
  names the failing call. Logical failures are `SyncError::new(key, debuginfo, retryable,
  offline)` (or `fatal` / `offline`). Constraint races on insert check
  `Error::is_constraint_violation()`. Workers sleep on retryable errors; the rest, task
  local-phase database errors and panics on an account's threads stop that account through
  `WorkerContext::fail` (`airpost_sync::runtime`), whose `sync`-mode handler logs critical and
  aborts; in `serve` mode it logs at error (nothing on stderr), stops only that account and
  sends `account.state` `stopped`. A panic on an engine-wide thread (stdin, socket, signals)
  is logged as critical and aborts in both modes.
- **Database.** One `Store` per thread (it is `!Send`). SQL is inline uppercase literals;
  repeated statements use `prepare_cached`. Writes that batch deltas run in
  `store.transaction("<function_name>", |s| …)`. The schema is `airpost_store::schema::V12_SCHEMA`
  (the C++ schema, pinned by `tests/fixtures/v12_schema.json`) plus
  `airpost_store::schema::MIGRATIONS`. A schema change is a new migration entry: bump
  `SCHEMA_VERSION` and add what it creates to `tests/fixtures/migrations.json`. Never edit a
  migration that has shipped.
- **Models** wrap their JSON (`data: Value`) and expose typed accessors over it, so keys
  the engine doesn't know survive a save. `Model::columns()` lists every bound column and
  `bind` binds them as `:<column>`.
- **Threads and async.** Store access is synchronous. A thread that speaks a network
  protocol owns a current-thread tokio runtime and blocks on it; never call a blocking HTTP
  client (`airpost-net`) from inside that runtime. Fetch OAuth tokens before entering it.
- **Portability.** No Unix-only APIs outside `src/platform.rs`. Build paths with `Path`,
  never by joining strings with `/`.
- **Branding.** Don't add Mailspring branding or calls to `*.getmailspring.com`. New accounts
  keep helper folders under `Airpost/`; the server-side names Mailspring created
  (`Mailspring/Snoozed`, `[Mailspring]/X`) are still recognised.
