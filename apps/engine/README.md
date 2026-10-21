# tern-engine

The Tern mail sync engine, a Rust port of Mailspring-Sync. One process (`--mode serve`) holds
every account and serves the desktop, `tern`, `tern-mcp` and `tern-gateway` over JSON-RPC on a
Unix socket (`run/engine.sock` in the profile; the protocol is `crates/tern-ipc`). It usually
runs as a launchd or systemd user unit that `tern engine install` writes (`--detached`, no
parent process). The older one-account-per-process `--mode sync` and the one-shot modes
(`test`, `reset`, `migrate`, ...) are still there.

See `CLAUDE.md` here for the crate layout, conventions and commands. The black-box test
harness is in `test/`.
