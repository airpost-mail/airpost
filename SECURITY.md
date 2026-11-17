# Security

## Reporting a vulnerability

Report vulnerabilities in Airpost privately through GitHub's private vulnerability reporting:
[github.com/airpost-mail/airpost/security/advisories/new](https://github.com/airpost-mail/airpost/security/advisories/new).
Airpost has no security email address. Please don't open a public issue for a vulnerability.

Include what you can: the affected component (desktop app, engine, CLI, `airpost-mcp`, server,
web client), the version or commit, steps to reproduce, and the impact you expect. Reports
about a download site that hosts modified or malicious builds under the Airpost name are welcome
through the same form.

## Scope

Everything in this repository. Third-party plugins and themes are fully trusted code once
installed ([docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md](docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md),
Trust Model), so a plugin doing what any local program can do is not a vulnerability in Airpost.

## How Airpost handles your data

The desktop app and its engine run on your machine. Mail account credentials are encrypted
with the OS keychain (Electron safeStorage, keychain item "Airpost Safe Storage") and are only
handed to the local engine. An Airpost server you sign in to gets an account's credentials only
when you choose to sync that account on the server too; there they are held by the server's
engine, never by its account or web layers (docs/architecture.md).
