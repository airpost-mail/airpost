# mailsync

The Tern mail sync engine, a Rust port of Mailspring-Sync. One process per account, driven
by the desktop client over stdin/stdout. See `docs/engine-port-plan.md` for the plan and
`CLAUDE.md` here for conventions and commands. The black-box test harness is in `test/`.
