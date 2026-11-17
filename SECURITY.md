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

The desktop app and its engine run on your machine. Mail account credentials are kept by the
local engine in the OS keychain (macOS Keychain or Linux Secret Service, item "Airpost Engine"),
or in an encrypted `secrets.enc` file on machines without a keychain. On a self-hosted Airpost
server (`deploy/server/`), the same engine runs headless and holds the credentials of the
accounts you add to it. The gateway and MCP reach it only through its socket, and the web client
only through the gateway (docs/architecture.md).
