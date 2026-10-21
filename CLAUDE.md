# CLAUDE.md

Monorepo for Tern, a fork of Mailspring (`apps/desktop`) and Mailspring-Sync (`legacy/mailsync-cpp`).

- Layout: deployables in `apps/` (desktop, engine, backend, web), Rust libraries in `crates/`, TS libraries in `packages/`. One Cargo workspace at the root covers `apps/engine`, `apps/backend` and `crates/*`.
- The Rust engine (`apps/engine`, `crates/`) is documented in `apps/engine/CLAUDE.md`.
- The desktop app is documented in `apps/desktop/CLAUDE.md`, with its docs (plugin system, theme tokens, localization, plans) in `docs/desktop/`. Claude hooks, skills and commands live in the root `.claude/`.
- `legacy/mailsync-cpp/CLAUDE.md` documents the C++ engine's design, invariants and conventions. C++ paths in comments (`MailSync/…`, `Vendor/…`, `test/…`) are relative to `legacy/mailsync-cpp/`.
- Licences: everything in the repo is GPL-3.0-only (not "or later"), including code written from scratch (`apps/backend`, `crates/tern-wire`, new `packages/`). Upstream Mailspring and Mailspring-Sync are received under GPL-3.0-or-later. A part may move to MIT later only by a deliberate decision, and only while all of its code is ours. New dependencies must be GPL-3.0-compatible (MIT, Apache-2.0, ISC, BSD, MPL-2.0). Rust files start with `// SPDX-License-Identifier: GPL-3.0-only`. Keep upstream copyright notices, and credit ported third-party code (mailcore2, libetpan, iCalendarLib) in the file and in NOTICE.md. Exception: the Tern name and logos (`assets/brand/` and their copies in `apps/desktop`) are trademarks, all rights reserved and outside the GPL (`assets/brand/LICENSE.md`); forks must replace them.
- Don't add new Mailspring branding or calls to `*.getmailspring.com`.
