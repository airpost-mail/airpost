# CLAUDE.md

Monorepo for Tern, a fork of Mailspring (`apps/desktop`) and Mailspring-Sync (`legacy/mailsync-cpp`).

- Layout: deployables in `apps/` (desktop, engine, backend), Rust libraries in `crates/`, TS libraries in `packages/`. One Cargo workspace at the root covers `apps/engine`, `apps/backend` and `crates/*`.
- The Rust engine port is specified in `docs/engine-port-plan.md`. Its decisions (D1–D12) are settled; work from that file.
- `legacy/mailsync-cpp/CLAUDE.md` documents the C++ engine's design, invariants and conventions. C++ paths in the plan (`MailSync/…`, `Vendor/…`, `test/…`) are relative to `legacy/mailsync-cpp/`.
- Licences: code derived from Mailspring or Mailspring-Sync (`apps/desktop`, `apps/engine`, the `ms-*` crates) is GPL-3.0-only (not "or later"). Code written from scratch with no GPL-derived code (`apps/backend`, `crates/ms-wire`, new `packages/`) is MIT, and must not depend on GPL crates. New dependencies must be GPL-compatible (MIT, Apache-2.0, ISC, BSD, MPL-2.0); MIT code takes only permissive ones. Keep upstream copyright notices.
- Don't add new Mailspring branding or calls to `*.getmailspring.com`.
