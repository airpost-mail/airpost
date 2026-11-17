# airpost-backend

The self-hostable Airpost backend: one binary, `airpost-backend`, with a SQLite database. It never
holds mail credentials or mail content (`docs/architecture.md`). Running it in production
(Docker Compose, systemd, configuration, backups, the reverse proxy) is in
the [Airpost docs site](https://docs.airpost.sh/self-hosting/overview).

## What it does today

- Users and per-device tokens, managed with `airpost-backend user …` and `airpost-backend token …`,
  and the Airpost login page (`/airpost/v1/login`, sign-up closed by default).
- `GET /airpost/v1/server` (server id, version, API version, features) and `GET /airpost/v1/me`.
- Metadata sync between devices, the protocol the engine's `HttpMetadataBackend`
  (`crates/airpost-net`) speaks: pushes, backfill, pages, the `/deltas/<aid>/streaming` delta
  stream, and expiration claims.
- Consistent backups (scheduled, manual and before each schema migration), `restore` and
  `rotate-server-id`, and `healthcheck` for container health checks.

Open and link tracking and sharing aren't implemented: `IMPLEMENTED_FEATURES` in
`src/http/mod.rs` has both off, so the server never advertises them. The desktop doesn't sign
in to a backend yet either (Preferences > Airpost server says it comes in a later version).

## Running it locally

From the repo root:

    export AIRPOST_DATA_DIR=/tmp/airpost-backend
    cargo run -p airpost-backend -- user add you@example.com --first-name You   # prints a password
    cargo run -p airpost-backend -- serve        # http://127.0.0.1:5101
    curl http://127.0.0.1:5101/airpost/v1/server

`airpost-backend --help` lists every command; `--config airpost.toml` (or `AIRPOST_CONFIG`) reads a
config file, with `AIRPOST_*` environment variables overriding it.

## Development

GPL-3.0-only, like the rest of the repo (see `LICENSE`). It is a member of the root Cargo
workspace (one `Cargo.lock`, target dir at the root) but depends on no engine `airpost-*` crate;
the wire types are in `crates/airpost-wire`. From the repo root:

    cargo test -p airpost-backend -p airpost-wire
    cargo clippy -p airpost-backend -p airpost-wire --all-targets -- -D warnings
    cargo deny --manifest-path apps/backend/Cargo.toml check   # uses apps/backend/deny.toml

Layout: `src/repo/` (SQLite schema, migrations, the `Repo` trait, the metadata version rules),
`src/http/` (routes, auth extractor, CORS; `login.rs`, `metadata.rs`, `stream.rs` and the
`hub.rs` fan-out), `src/cli.rs` (commands), `src/backup.rs`, `tests/` (integration tests
against a server on an ephemeral port; `tests/metadata.rs` reads the stream the way
`MetadataWorker.cpp` does, on paused tokio time). The engine's `HttpMetadataBackend` is tested
against a real server from the engine side, in `crates/airpost-net/tests/backend_server.rs`, so
this crate needs no engine crate, even as a dev dependency.
