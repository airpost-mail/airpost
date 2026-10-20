# Security

## Reporting a vulnerability

Report vulnerabilities in Tern privately through GitHub's private vulnerability reporting:
[github.com/ternmail/ternmail/security/advisories/new](https://github.com/ternmail/ternmail/security/advisories/new).
Tern has no security email address. Please don't open a public issue for a vulnerability.

Include what you can: the affected component (desktop app, engine, CLI, `tern-mcp`, server,
web client), the version or commit, steps to reproduce, and the impact you expect. Reports
about a download site that hosts modified or malicious builds under the Tern name are welcome
through the same form.

## Scope

Everything in this repository. `legacy/mailsync-cpp` is the C++ engine kept as a reference;
Tern doesn't ship it, so a finding there matters only if the Rust engine shares it.
Third-party plugins and themes are fully trusted code once installed
([docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md](docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md),
Trust Model), so a plugin doing what any local program can do is not a vulnerability in Tern.

## How Tern handles your data

The desktop app and its engine run on your machine. Mail account credentials are encrypted
with the OS keychain (Electron safeStorage, keychain item "Tern Safe Storage") and are only
handed to the local engine. A Tern server you sign in to gets an account's credentials only
when you choose to sync that account on the server too; there they are held by the server's
engine, never by its account or web layers (docs/architecture.md, docs/product-model.md).
