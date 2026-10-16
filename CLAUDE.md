# CLAUDE.md

Monorepo for Tern, a fork of Mailspring (`apps/desktop`) and Mailspring-Sync (`legacy/mailsync-cpp`).

- Layout: deployables in `apps/` (desktop, engine, backend, web), Rust libraries in `crates/`, TS libraries in `packages/`. One Cargo workspace at the root covers `apps/engine`, `apps/backend` and `crates/*`.
- The Rust engine port is specified in `docs/engine-port-plan.md`. Its decisions (D1–D12) are settled; work from that file.
- `legacy/mailsync-cpp/CLAUDE.md` documents the C++ engine's design, invariants and conventions. C++ paths in the plan (`MailSync/…`, `Vendor/…`, `test/…`) are relative to `legacy/mailsync-cpp/`.
- Licences: everything in the repo is GPL-3.0-only (not "or later"), including code written from scratch (`apps/backend`, `crates/ms-wire`, new `packages/`). Upstream Mailspring and Mailspring-Sync are received under GPL-3.0-or-later. A part may move to MIT later only by a deliberate decision, and only while all of its code is ours. New dependencies must be GPL-3.0-compatible (MIT, Apache-2.0, ISC, BSD, MPL-2.0). Rust files start with `// SPDX-License-Identifier: GPL-3.0-only`. Keep upstream copyright notices, and credit ported third-party code (mailcore2, libetpan, iCalendarLib) in the file and in NOTICE.md.
- Don't add new Mailspring branding or calls to `*.getmailspring.com`.
