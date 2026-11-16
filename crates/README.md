# crates

Rust library crates, all GPL-3.0-only (see the root LICENSE.md and NOTICE.md). Most are the
engine, split up: they are used by `apps/engine` only. `airpost-ipc`, `airpost-client`,
`airpost-compose` and `airpost-activation` are also used by the programs that talk to the engine
(`apps/cli`, `apps/mcp`, `apps/gateway`), and `airpost-wire` is shared by the engine and
`apps/backend`. `apps/engine/CLAUDE.md` has the dependency order.

| Crate | |
|---|---|
| `airpost-activation` | What a program run by a user unit needs: the listening socket launchd or systemd passed it, and the build it runs. Used by `airpost-gateway` and `airpost-mcp`. |
| `airpost-ai` | Model providers for thread summaries, reply suggestions, reply drafts and inbox triage: OpenAI-compatible, Anthropic and Ollama APIs. |
| `airpost-client` | The client side of the engine socket: discovery, profile resolution, hello, the multi-engine router, typed calls and task builders. Used by `airpost`, `airpost-mcp` and `airpost-gateway`. |
| `airpost-compose` | Drafts as the desktop builds them: new, reply, forward and mailto drafts, quoting, signatures, send warnings. |
| `airpost-core` | Errors, ids, constants, logging, the clock, the account and identity, header and flag types, UID sets and wake signals, shared by every engine crate. |
| `airpost-dav` | CalDAV and CardDAV protocol helpers: XML, vCard, iCalendar fields and discovery. |
| `airpost-imap` | The IMAP session the sync workers and tasks drive (from mailcore2 and libetpan). |
| `airpost-integrations` | Third-party providers for translation (LibreTranslate, DeepL), grammar checks (LanguageTool) and contact enrichment (People Data Labs). |
| `airpost-ipc` | The engine's local socket protocol: JSON-RPC 2.0 over newline-delimited JSON on a Unix socket, method types and a blocking client. Depends on serde and serde_json only. |
| `airpost-mime` | MIME parsing, body rendering, snippets and the outgoing message builder. |
| `airpost-net` | HTTP helpers, OAuth token refresh, the metadata backends (local only, or a Airpost backend) and the Google People client. |
| `airpost-pim` | Calendar and contact editing: ICS edits, recurrence expansion, RSVP and counter-proposals, vCard mapping, VCF import and export. |
| `airpost-providers` | Server and provider quirk profiles, folder-role tables and OAuth endpoints. |
| `airpost-secrets` | The engine's secret store: the OS keychain, Secret Service, or an AES-256-GCM file. |
| `airpost-smtp` | SMTP sending and the test-mode SMTP check. |
| `airpost-store` | The engine's SQLite store: schema, models, queries, transactions, placements and the delta stream. |
| `airpost-sync` | The sync workers: background scans, foreground IDLE, mail processing, metadata and calendar/contact threads. |
| `airpost-tasks` | Client tasks: the local and remote phases of every task a client queues. |
| `airpost-wire` | Wire types shared by `airpost-backend` and the engine's HTTP metadata backend. Depends on serde and serde_json only, and on no engine crate. |
