# Tern

A desktop email client with a sync engine written in Rust, forked from Mailspring.

| Path | Contents |
|---|---|
| `apps/desktop/` | Electron client |
| `apps/engine/` | Rust sync engine binary (`tern-engine`); see `apps/engine/README.md` |
| `apps/backend/` | Self-hostable backend for metadata sync, tracking and sharing, not started |
| `apps/web/` | Web client (Vite, React, shadcn) for a hosted engine; runs on a mock engine for now, see `apps/web/TODO-engine.md` |
| `crates/` | Rust libraries shared by the engine and backend |
| `packages/` | TS libraries shared across apps |
| `legacy/mailsync-cpp/` | The C++ sync engine: the reference and A/B baseline until the Rust engine reaches parity |

To build and run from source, see CONTRIBUTING.md; to report a vulnerability, SECURITY.md.

Licensed under GPL-3.0-only; see LICENSE.md and NOTICE.md.
