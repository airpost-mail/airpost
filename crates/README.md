# crates

Rust library crates, all GPL-3.0-only (see the root LICENSE.md and NOTICE.md). Most are the
engine, split up: they are used by `apps/engine` only. `tern-ipc`, `tern-client`,
`tern-compose` and `tern-activation` are also used by the programs that talk to the engine
(`apps/cli`, `apps/mcp`, `apps/gateway`), and `tern-wire` is shared by the engine and
`apps/backend`. `apps/engine/CLAUDE.md` has the dependency order.

| Crate | |
|---|---|
| `tern-activation` | What a program run by a user unit needs: the listening socket launchd or systemd passed it, and the build it runs. Used by `tern-gateway` and `tern-mcp`. |
| `tern-ai` | Model providers for thread summaries, reply suggestions, reply drafts and inbox triage: OpenAI-compatible, Anthropic and Ollama APIs. |
| `tern-client` | The client side of the engine socket: discovery, profile resolution, hello, the multi-engine router, typed calls and task builders. Used by `tern`, `tern-mcp` and `tern-gateway`. |
| `tern-compose` | Drafts as the desktop builds them: new, reply, forward and mailto drafts, quoting, signatures, send warnings. |
| `tern-core` | Errors, ids, constants, logging, the clock, the account and identity, header and flag types, UID sets and wake signals, shared by every engine crate. |
| `tern-dav` | CalDAV and CardDAV protocol helpers: XML, vCard, iCalendar fields and discovery. |
| `tern-imap` | The IMAP session the sync workers and tasks drive (from mailcore2 and libetpan). |
| `tern-integrations` | Third-party providers for translation (LibreTranslate, DeepL), grammar checks (LanguageTool) and contact enrichment (People Data Labs). |
| `tern-ipc` | The engine's local socket protocol: JSON-RPC 2.0 over newline-delimited JSON on a Unix socket, method types and a blocking client. Depends on serde and serde_json only. |
| `tern-mime` | MIME parsing, body rendering, snippets and the outgoing message builder. |
| `tern-net` | HTTP helpers, OAuth token refresh, the metadata backends (local only, or a Tern backend) and the Google People client. |
| `tern-pim` | Calendar and contact editing: ICS edits, recurrence expansion, RSVP and counter-proposals, vCard mapping, VCF import and export. |
| `tern-providers` | Server and provider quirk profiles, folder-role tables and OAuth endpoints. |
| `tern-secrets` | The engine's secret store: the OS keychain, Secret Service, or an AES-256-GCM file. |
| `tern-smtp` | SMTP sending and the test-mode SMTP check. |
| `tern-store` | The engine's SQLite store: schema, models, queries, transactions, placements and the delta stream. |
| `tern-sync` | The sync workers: background scans, foreground IDLE, mail processing, metadata and calendar/contact threads. |
| `tern-tasks` | Client tasks: the local and remote phases of every task a client queues. |
| `tern-wire` | Wire types shared by `tern-backend` and the engine's HTTP metadata backend. Depends on serde and serde_json only, and on no engine crate. |
