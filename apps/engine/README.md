# tern-engine

The Tern mail sync engine, a Rust port of Mailspring-Sync. One `serve` process runs every
account of a profile; the desktop starts it and talks to it over stdin/stdout and a Unix
socket, which the CLI, `tern-mcp` and `tern-gateway` use too. See `CLAUDE.md` here for
conventions and commands. The black-box test harness is in `test/`.
