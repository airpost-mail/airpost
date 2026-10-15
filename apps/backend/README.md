# tern-backend

Self-hostable Tern server: identity and device tokens, `/onboarding`, `/api/me`, metadata sync
(push, backfill, pages, the delta stream) and expiration claims; tracking and sharing follow in B2. The spec is `docs/backend-plan.md`; running it is in
`docs/self-hosting.md`.

This crate has its own `[workspace]`, `Cargo.lock` and `target/` until B1-E joins it to the
root workspace (plan 3.1). Run cargo from this directory:

    cargo test
    cargo clippy --all-targets -- -D warnings
    cargo deny check

Layout: `src/wire.rs` (wire types, moves to `crates/ms-wire` in B1-E), `src/repo/` (SQLite
schema, migrations, the `Repo` trait, the metadata version rules), `src/http/` (routes, auth
extractor, CORS; `metadata.rs`, `stream.rs` and the `hub.rs` fan-out for 3.5), `src/cli.rs`
(commands), `tests/` (integration tests against a server on an ephemeral port;
`tests/metadata.rs` reads the stream the way `MetadataWorker.cpp` does, on paused tokio time).
