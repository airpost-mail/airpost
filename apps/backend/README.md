# tern-backend

Self-hostable Tern server: users and device tokens, the Tern login page, `/tern/v1/server`,
`/tern/v1/me`, metadata sync (push, backfill, pages, the delta stream) and expiration claims;
tracking and sharing follow in B2. The spec is `docs/backend-plan.md`; running it is in
`docs/self-hosting.md`.

GPL-3.0-only, like the rest of the repo (see `LICENSE`). It is a member of the root Cargo
workspace (one `Cargo.lock`, target dir at the root) but depends on no engine `tern-*` crate; the
wire types are in `crates/tern-wire`. From the repo root:

    cargo test -p tern-backend -p tern-wire
    cargo clippy -p tern-backend -p tern-wire --all-targets -- -D warnings
    cargo deny --manifest-path apps/backend/Cargo.toml check   # uses apps/backend/deny.toml

Layout: `src/repo/` (SQLite
schema, migrations, the `Repo` trait, the metadata version rules), `src/http/` (routes, auth
extractor, CORS; `login.rs` for 3.4; `metadata.rs`, `stream.rs` and the `hub.rs` fan-out for 3.5), `src/cli.rs`
(commands), `tests/` (integration tests against a server on an ephemeral port;
`tests/metadata.rs` reads the stream the way `MetadataWorker.cpp` does, on paused tokio time).
The engine's `HttpMetadataBackend` is tested against a real server from the engine side, in
`crates/tern-net/tests/backend_server.rs`, so this crate needs no engine crate, even as a dev
dependency.
