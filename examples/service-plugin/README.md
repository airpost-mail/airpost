# Sample service plugin

A Airpost plugin whose logic runs as a service: a separate program on the engine socket that
launchd (macOS) or systemd `--user` (Linux) runs after the engine, also while the desktop is
closed (docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md, Service Plugins).

It is a "remind me" plugin: when a thread's `airpost-sample-service` metadata reaches its
`expiration`, the service stars the thread, marks it unread and clears the expiration. Other
plugins' due rows are left alone.

## Try it

```sh
cargo build --release -p airpost-sample-service
mkdir -p examples/service-plugin/bin
cp target/release/airpost-sample-service examples/service-plugin/bin/
```

In Airpost, Preferences > Plugins > "Install a plugin…", pick `examples/service-plugin`, then
"Install service…" on it. The pane shows the binary, its SHA-256 and the units before it
writes them. From a terminal the same is:

```sh
airpost service install airpost-sample-service --dry-run   # what would be written
airpost service install airpost-sample-service
airpost service list
airpost service uninstall airpost-sample-service
```

Give a thread a reminder (one minute from now) and quit Airpost:

```sh
airpost task queue '{"__cls": "SyncbackMetadataTask", "modelId": "<thread id>",
  "modelClassName": "thread", "pluginId": "airpost-sample-service",
  "value": {"expiration": '$(( $(date +%s) + 60 ))'}}'
```

Its log is `<profile>/services/airpost-sample-service/service.log` on macOS and
`journalctl --user -u airpost-service-airpost-sample-service` on Linux.

Run it by hand against a running engine with
`airpost-sample-service --config-dir <profile>`.
