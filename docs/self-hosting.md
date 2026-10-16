# Self-hosting the Tern backend

The backend (`apps/backend`, one binary, `tern-backend`) adds the features that need a
server on the internet: metadata sync between your devices (snoozes, reminders, tracking
state), open and link tracking, and, once they land, sharing and signature images. It never
holds mail credentials or mail content; those stay in the engine on your devices
(`docs/architecture.md`). Tern works without it.

It serves plain HTTP and expects a TLS reverse proxy in front. The compose file below
includes one (Caddy), which gets a certificate for your domain.

Not yet released: the published image (`ghcr.io/ternmail/tern-backend`) and the desktop's
server settings. Until the image exists, build it from a checkout (step 3 below shows how).

## With Docker Compose

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

## Without Docker

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

## Configuration

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

## Users and tokens

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

## Backups and restores

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

## Upgrades

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

## Reverse proxy

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

## Health and logs

- `GET /ping` answers `200 {}`. `tern-backend healthcheck` calls it on the bind address and
  exits non-zero when it gets no `200`; the compose file uses it, since the image has no
  shell or curl.
- Logs go to stderr: `docker compose logs backend` or `journalctl -u tern-backend`.

## Uninstall

Sign out on each device first, which revokes its token; the desktop then keeps working
without a server. Then:

    docker compose down          # add -v to delete the database, backups and certificates

Without Docker: `systemctl disable --now tern-backend`, remove the binary, the unit,
`/etc/tern-backend` and `/var/lib/tern-backend`.

What stops working for other people: tracking pixels and tracked links in mail you already
sent point at this server, and so will shared pages and hosted signature images.

## Licence

`tern-backend` is GPL-3.0-only (`apps/backend/LICENSE`). Running it, for yourself or for other
people over the network, puts no obligation on you. If you pass the binary or an image to
someone else, give them the licence, the third-party notices and the corresponding source
(GPLv3 section 6), for example a link to the tagged source next to the download.
