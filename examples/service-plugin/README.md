# Sample service plugin

A Tern plugin whose logic runs as a service: a separate program on the engine socket that
launchd (macOS) or systemd `--user` (Linux) runs after the engine, also while the desktop is
closed (docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md, Service Plugins).

It is a "remind me" plugin: when a thread's `tern-sample-service` metadata reaches its
`expiration`, the service stars the thread, marks it unread and clears the expiration. Other
plugins' due rows are left alone.

## Try it

```sh
cargo build --release -p tern-sample-service
mkdir -p examples/service-plugin/bin
cp target/release/tern-sample-service examples/service-plugin/bin/
```

In Tern, Preferences > Plugins > "Install a plugin…", pick `examples/service-plugin`, then
"Install service…" on it. The pane shows the binary, its SHA-256 and the units before it
writes them. From a terminal the same is:

```sh
tern service install tern-sample-service --dry-run   # what would be written
tern service install tern-sample-service
tern service list
tern service uninstall tern-sample-service
```

Give a thread a reminder (one minute from now) and quit Tern:

```sh
tern task queue '{"__cls": "SyncbackMetadataTask", "modelId": "<thread id>",
  "modelClassName": "thread", "pluginId": "tern-sample-service",
  "value": {"expiration": '$(( $(date +%s) + 60 ))'}}'
```

Its log is `<profile>/services/tern-sample-service/service.log` on macOS and
`journalctl --user -u tern-service-tern-sample-service` on Linux.

Run it by hand against a running engine with
`tern-sample-service --config-dir <profile>`.
