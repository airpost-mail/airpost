# Self-hosting Tern

Two things can run on your server:

- **The Tern server** (`deploy/server/`, the `tern-server` image): the engine running
  headless, so your mail syncs and scheduled sends go out while every device is off, with
  Tern on the web, the backend and MCP for agents next to it, each switched on or off.
- **The backend alone** (`deploy/backend/`, the `tern-backend` image): metadata sync between
  your desktops and open and link tracking, without mail on the server.

## Tern server

`deploy/server/compose.yaml` runs one service per component, all from the same image:

| Service | Turned on by | Default | |
|---|---|---|---|
| `engine` | always | on | `tern-engine --mode serve --detached`: syncs the accounts, keeps their credentials |
| `gateway` | `web` | on | `tern-gateway` with the web client, paired browsers only |
| `caddy` | `proxy` | on | TLS for `TERN_DOMAIN`; leave it out behind your own proxy |
| `backend` | `backend` | off | `tern-backend` (below): metadata sync between desktops, tracking. The web client doesn't need it. |
| `mcp` | `mcp` | off | `tern-mcp http` at `https://<domain>/mcp`: agents reach your mail with a bearer token, so it is opt-in |

They stay separate processes in separate containers. Docker restarts one that stops; there is
no supervisor. The image also has `tern`, the CLI, for admin inside the engine's container.
The image has no shell.

Not yet released: the published image (`ghcr.io/ternmail/tern-server`). Until it exists, build
it from a checkout (step 3).

### Setup

You need a Linux machine with Docker, a DNS name pointing at it, and ports 80 and 443 open.

1. Get `deploy/server/` from the repository and change into it.
2. `cp .env.example .env` and set `TERN_DOMAIN`, `TERN_VERSION` and `COMPOSE_PROFILES`
   (default `web,proxy`; add `backend` and `mcp` to run them).
3. Start it:

       docker compose up -d

   From a checkout, before images are published:

       TERN_VERSION=dev docker compose -f compose.yaml -f compose.build.yaml up -d --build

4. `docker compose ps` shows each service `healthy`. `docker compose exec engine tern status`
   lists the engine and its accounts (none yet).

To try it on one machine without a domain: `TERN_DOMAIN=127.0.0.1`,
`TERN_PUBLIC_URL=http://127.0.0.1:5180`, `COMPOSE_PROFILES=web`, and
`docker compose -f compose.yaml -f compose.own-proxy.yaml up -d`; the web client is then at
`http://127.0.0.1:5180`.

To turn a component on, add its profile to `COMPOSE_PROFILES` and `docker compose up -d`. To
turn one off, take it out and remove its container: `docker compose rm -sf mcp`.

Behind a proxy you already run, leave `proxy` out and add `-f compose.own-proxy.yaml`: the
gateway is on `127.0.0.1:5180`, the backend on `127.0.0.1:5101` and MCP on `127.0.0.1:2587`.
Route as `deploy/server/Caddyfile` does: `/ping`, `/tern/*`, `/metadata/*`, `/deltas/*` and
`/t/*` to the backend (unbuffered, see [Reverse proxy](#reverse-proxy)), `/mcp` to MCP
(unbuffered), everything else to the gateway with WebSocket upgrades and the original `Host`
(the gateway's README has an nginx block).

### Adding mail accounts

From the engine's container, with the `tern` CLI. An IMAP account (Tern finds the servers
for most providers; `--imap-host`, `--smtp-host` and the other flags of
`tern account add --help` override them):

    docker compose exec engine tern account add you@example.com

It asks for the password (or reads it with `--password-stdin`), tests the account, and
stores it. Gmail and Microsoft accounts sign in in a browser, which on a server runs on
another computer:

    docker compose exec engine tern account add you@gmail.com --provider gmail --paste

Open the address it prints, sign in, and paste back the `http://127.0.0.1:12141/?code=…`
address the browser ends on (the page itself doesn't load). `--provider office365` and
`outlook` work the same way.

A paired admin browser can also add IMAP accounts in the web client's Settings > Accounts.

`docker compose exec engine tern status` shows each account's state;
`docker compose exec engine tern logs <email> -f` follows its log.

### Pairing a browser

    docker compose exec gateway tern-gateway pair            # mail only
    docker compose exec gateway tern-gateway pair --admin    # also accounts and settings

It prints a code and a `https://<domain>/pair#<code>` link, valid once for 10 minutes. Open
the link on the device. `tern-gateway devices list` and `devices revoke <id>` (with the same
`docker compose exec gateway` prefix) manage paired browsers. The gateway's README has the
details.

### MCP

With `mcp` in `COMPOSE_PROFILES`, agents connect to `https://<domain>/mcp` with the token
from `docker compose exec mcp tern-mcp token`. `deploy/server/mcp.json` is the policy, mounted
read-only: `accessLevel` starts at `read-only`; `read-write` and `read-write-send` and the
per-account switches are in `docs/mcp.md`. Edit the file, and the next call reads it. Every
call is logged to `mcp-audit.log` in the profile.

### Mail credentials

The engine keeps each account's settings in `accounts.json` and its passwords and refresh
tokens, with integration and AI keys, in `secrets.enc`, both in the `profile` volume.
`secrets.enc` is encrypted (AES-256-GCM) with a key in `/keys/engine.key`, on the `engine-key`
volume. The engine creates the key when it stores its first secret.

Only the engine's container mounts `engine-key`. The gateway and MCP containers mount
`profile`, because the engine's socket is there, so they can read the mail database (which
they serve anyway) but not the credentials. What the key file doesn't protect against: root
on the host, anyone holding both volumes or a backup of both, and a compromised gateway or
MCP container acting through the socket, which takes every connection from the profile's
user as the owner's. A keychain unlocked by a
passphrase would, at the price of typing it after every restart; the server doesn't offer one
yet.

To keep the key off the server's disks, mount your own instead of the volume: a file of at
least 32 random bytes, owned by uid 65532 and mode 0600 (`openssl rand -hex 32`), for example
from a secrets manager's tmpfs. With a different or lost key the engine can't read
`secrets.enc`: accounts show `ErrorAccountNotStarted` and nothing is overwritten. Put the
right key back, or delete `secrets.enc` and `accounts.json` and add the accounts again.

### Backups

- `profile`: the mail database, settings, signatures, rules, the accounts and their
  encrypted credentials, paired browsers, the MCP token. Mail itself stays on your mail
  servers; without this volume you add the accounts again and Tern syncs from scratch.
- `engine-key`: the key for the credentials. Back it up somewhere other than `profile`'s
  backups.
- `backend-data` and `backend-backups`, with the backend: [Backups and restores](#backups-and-restores).

For a consistent copy of the profile, stop the engine first:

    docker compose stop engine
    docker run --rm -v tern-server_profile:/p:ro -v "$PWD":/out alpine \
        tar czf /out/tern-profile.tgz -C /p .
    docker compose start engine

### Upgrades

Set the new `TERN_VERSION` in `.env`, then

    docker compose pull
    docker compose up -d

The engine migrates its database when it starts and doesn't keep a copy of the old one, so
back up `profile` first if you might roll back. The backend copies its own database before
a migration.

### Health and logs

Each service's health check is a command in the image: `tern status` for the engine (it exits
3 when no engine answers), `tern-gateway healthcheck`, `tern-backend healthcheck` and
`tern-mcp healthcheck`. The engine logs into the profile, not to Docker:
`docker compose exec engine tern logs <email>` for an account. The gateway, backend and MCP
log to `docker compose logs <service>`.

## Backend only

The backend (`apps/backend`, one binary, `tern-backend`) adds the features that need a
server on the internet: metadata sync between your devices (snoozes, reminders, tracking
state), open and link tracking, and, once they land, sharing and signature images. The backend
process never holds mail credentials or mail content; those stay in the engine, on your
devices or in the Tern server's engine container (`docs/architecture.md`). Tern works without
it. In the Tern server it is the `backend` service, configured by the same variables as
below.

It serves plain HTTP and expects a TLS reverse proxy in front. The compose file below
includes one (Caddy), which gets a certificate for your domain.

Not yet released: the published image (`ghcr.io/ternmail/tern-backend`) and the desktop's
server settings. Until the image exists, build it from a checkout (step 3 below shows how).

### With Docker Compose

You need a Linux machine with Docker, a DNS name pointing at it, and ports 80 and 443 open.

1. Get `deploy/backend/` from the repository (or the release's
   `tern-backend-compose.tar.gz` once releases exist) and change into it.
2. `cp .env.example .env` and set `TERN_DOMAIN` (for example `tern.example.com`) and
   `TERN_VERSION`.
3. Start it:

       docker compose up -d

   From a checkout, before images are published:

       TERN_VERSION=dev docker compose -f compose.yaml -f compose.build.yaml up -d --build

4. Create your user. The password is printed once:

       docker compose exec backend tern-backend user add you@example.com --first-name You

5. Check it from outside: `curl https://tern.example.com/tern/v1/server` returns the server
   id, version and features.
6. In Tern on each device, enter `https://tern.example.com` as the server and sign in. Each
   device gets its own token.

`docker compose ps` shows the backend as `healthy` once it answers `/ping`. Logs:
`docker compose logs -f backend`.

### Your own reverse proxy

To skip Caddy and publish the backend on `127.0.0.1:5101` for a proxy already running on the
host:

    docker compose -f compose.yaml -f compose.own-proxy.yaml up -d backend

Then configure the proxy as in [Reverse proxy](#reverse-proxy).

### Without Docker

Build the binary (`cargo build --release -p tern-backend`, giving
`target/release/tern-backend`) or take it from a release. Then, as root:

    useradd --system --home-dir /var/lib/tern-backend --shell /usr/sbin/nologin tern
    install -m 755 tern-backend /usr/local/bin/
    install -d -m 755 /etc/tern-backend
    install -m 640 -g tern deploy/systemd/tern.toml.example /etc/tern-backend/tern.toml
    $EDITOR /etc/tern-backend/tern.toml          # public_url at least
    install -m 644 deploy/systemd/tern-backend.service /etc/systemd/system/
    systemctl enable --now tern-backend
    sudo -u tern tern-backend --config /etc/tern-backend/tern.toml user add you@example.com

Run every admin command as the `tern` user with `--config`, so files it creates stay
readable by the server. Put a TLS proxy in front ([Reverse proxy](#reverse-proxy)).

### Configuration

`tern-backend --config tern.toml …` (or `TERN_CONFIG`). Environment variables override the
file.

| Key | Env | Default | |
|---|---|---|---|
| `data_dir` | `TERN_DATA_DIR` | `/data` | Holds `tern.db` (SQLite, WAL). |
| `bind` | `TERN_BIND` | `127.0.0.1:5101` | The image sets `0.0.0.0:5101`. |
| `public_url` | `TERN_PUBLIC_URL` | `http://<bind>` | Where clients and mail recipients reach the server. Required when binding `0.0.0.0`. Must be `https` unless it is a loopback address (tokens travel in `Authorization: Basic`); `serve --insecure-public-url` overrides that. |
| `signup` | `TERN_SIGNUP` | `closed` | `open` lets anyone create an account on `/tern/v1/login`. |
| `[features] tracking`, `sharing` | | `true` | Server-side switches; advertised only once implemented. |
| `trusted_proxies` | `TERN_TRUSTED_PROXIES` | `["127.0.0.0/8", "::1"]` | Peers whose `X-Forwarded-For` is used to find the client address for login rate limits. Comma-separated in the env var; empty trusts none. The compose file sets it to its own network. |
| `[backup] dir` | `TERN_BACKUP_DIR` | unset | Turns on scheduled backups into this directory. |
| `[backup] interval` | `TERN_BACKUP_INTERVAL` | `24h` | `<n>m`, `<n>h` or `<n>d`; at least `1m`. |
| `[backup] keep` | `TERN_BACKUP_KEEP` | `7` | Scheduled backups to keep; older ones are deleted. |
| | `TERN_LOG_FORMAT` | `text` | `json` for one JSON object per line. `RUST_LOG` sets the level (default `info`). |

### Users and tokens

    tern-backend user add you@example.com --first-name You     # prints a generated password
    tern-backend user add you@example.com --password-stdin < pw.txt
    tern-backend user list | disable | enable | delete --yes <email>
    tern-backend token create <email> [--name laptop]           # prints a device token
    tern-backend token list [--user <email>]
    tern-backend token revoke <id>

With Docker, prefix them with `docker compose exec backend`. While there are no users and
signup is closed, the server logs a reminder at startup.

Signing in from the desktop mints one token per device; revoke it to sign that device out.
Don't share one token between two running clients: a new metadata stream from a token
replaces that token's previous stream for the same account.

### Backups and restores

The database holds everything: users, password and token hashes, metadata, tracking events.
Every backup is a consistent copy taken while the server runs (`VACUUM INTO`), owner-only,
with a new server id (see restores below).

- **Scheduled.** With `[backup] dir` set, `serve` writes `tern-<UTC time>.db` there every
  `interval` and keeps the newest `keep`. The first one is taken at startup when the newest
  existing backup is older than the interval. Only files with that name pattern are ever
  deleted. The compose file turns this on (`backups` volume, daily, 14 kept); copy them off
  the machine with `docker compose cp backend:/backups ./backups`.
- **Manual.** `tern-backend backup /path/to/copy.db`.
- **Before a migration.** When an upgrade changes the schema, the server first writes
  `pre-migrate-<from>-to-<to>-<time>.db` into the data dir. Delete it once the new version
  runs well.

To restore, stop the server and run `tern-backend restore <file>`. It checks the file is a
backend database this version can open, replaces `tern.db` (removing any leftover `-wal`
and `-shm`), and gives it a new server id. It refuses to run while the server is up. With
Docker, for a backup still in the `backups` volume:

    docker compose stop backend
    docker compose run --rm backend restore /backups/tern-20261016T030000Z.db
    docker compose start backend

For a file from elsewhere, copy it into the volume first:
`docker compose cp ./tern-20261016T030000Z.db backend:/backups/`.

Clients see the new server id and re-sync their metadata from scratch, which keeps them from
skipping writes made after the backup was taken.

If you restore the data dir some other way (a filesystem snapshot), run
`tern-backend rotate-server-id` with the server stopped before starting it.

### Upgrades

With Compose: set the new `TERN_VERSION` in `.env`, then

    docker compose pull
    docker compose up -d

Without Docker: replace the binary and `systemctl restart tern-backend`.

The server applies schema migrations when it starts, after the pre-migration copy above. It
refuses to start on a database newer than itself, so to roll back after a migration, restore
the `pre-migrate-…` copy and run the previous version.

Desktops and servers of different versions work together: the `/tern/v1` API and the metadata
protocol only change in backward-compatible ways, and clients enable features from the
`features` the server reports, so a newer desktop on an older server shows the missing
feature as unavailable. `GET /tern/v1/server` reports `version` and `apiVersion`; a breaking
change would come as `/tern/v2` next to `v1`.

### Reverse proxy

The metadata stream (`/deltas/<aid>/streaming`) is a long-lived chunked response with a
16-byte heartbeat every 5 s, and clients drop it after 30 s without data. A proxy that
buffers responses breaks it, so turn buffering off.

Caddy:

    tern.example.com {
        reverse_proxy 127.0.0.1:5101 {
            flush_interval -1
        }
    }

nginx:

    location / {
        proxy_pass http://127.0.0.1:5101;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    }
    location /deltas/ {
        proxy_pass http://127.0.0.1:5101;
        proxy_http_version 1.1;
        proxy_buffering off;
        proxy_read_timeout 1h;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    }

The server sends `X-Accel-Buffering: no` on the stream, but that only covers nginx's defaults.

Login is rate limited per client IP and per email. The client IP comes from `X-Forwarded-For`
when the connection is from a `trusted_proxies` address (Caddy sets the header by default;
nginx needs the line above). If the proxy is not trusted, every login shares the proxy's
budget. Only trust addresses that are really your proxy: anyone else connecting from them
could pick their own rate-limit key.

### Health and logs

- `GET /ping` answers `200 {}`. `tern-backend healthcheck` calls it on the bind address and
  exits non-zero when it gets no `200`; the compose file uses it, since the image has no
  shell or curl.
- Logs go to stderr: `docker compose logs backend` or `journalctl -u tern-backend`.

### Uninstall

Sign out on each device first, which revokes its token; the desktop then keeps working
without a server. Then:

    docker compose down          # add -v to delete the database, backups and certificates

Without Docker: `systemctl disable --now tern-backend`, remove the binary, the unit,
`/etc/tern-backend` and `/var/lib/tern-backend`.

What stops working for other people: tracking pixels and tracked links in mail you already
sent point at this server, and so will shared pages and hosted signature images.

### Licence

`tern-backend` is GPL-3.0-only (`apps/backend/LICENSE`). Running it, for yourself or for other
people over the network, puts no obligation on you. If you pass the binary or an image to
someone else, give them the licence, the third-party notices and the corresponding source
(GPLv3 section 6), for example a link to the tagged source next to the download. The same
holds for the `tern-server` image; its notices are in `/usr/share/doc/tern`.
