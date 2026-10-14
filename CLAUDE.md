# CLAUDE.md

Monorepo for Tern, a fork of Mailspring (`apps/desktop`) and Mailspring-Sync (`legacy/mailsync-cpp`).

- The Rust engine port is specified in `docs/engine-port-plan.md`. Its decisions (D1–D11) are settled; work from that file.
- `legacy/mailsync-cpp/CLAUDE.md` documents the C++ engine's design, invariants and conventions. Paths in the plan that point at C++ sources (`MailSync/…`, `Vendor/…`, `test/…`) are relative to `legacy/mailsync-cpp/`.
- Everything is GPL-3.0. New dependencies must be GPL-compatible (MIT, Apache-2.0, ISC, BSD, MPL-2.0). Keep upstream copyright notices.
- Don't add new Mailspring branding or calls to `*.getmailspring.com`.
