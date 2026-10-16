# Self-hosting the Tern backend

Stub; the full guide (backups, upgrades, examples) lands with B3 of `docs/backend-plan.md`.
The backend is `apps/backend`, one binary, `tern-backend`.

## Run it

The server speaks plain HTTP and expects a TLS reverse proxy in front of it. Tokens travel in
`Authorization: Basic`, so `serve` refuses a `public_url` that is plain `http` on a
non-loopback host unless you pass `--insecure-public-url`.

Docker:

    docker build -f apps/backend/Dockerfile -t tern-backend .
    docker run -d --name tern -v tern-data:/data -p 127.0.0.1:5101:5101 \
        -e TERN_PUBLIC_URL=https://tern.example.com tern-backend

Without Docker:

    cargo build --release -p tern-backend
    TERN_DATA_DIR=/var/lib/tern TERN_PUBLIC_URL=https://tern.example.com \
        target/release/tern-backend serve

## Configuration

`tern-backend --config tern.toml …` (or `TERN_CONFIG`). Environment variables override the file.

| Key | Env | Default | |
|---|---|---|---|
| `data_dir` | `TERN_DATA_DIR` | `/data` | Holds `tern.db` (SQLite, WAL). |
| `bind` | `TERN_BIND` | `127.0.0.1:5101` | The image sets `0.0.0.0:5101`. |
| `public_url` | `TERN_PUBLIC_URL` | `http://<bind>` | Where clients and mail recipients reach the server. Required when binding `0.0.0.0`. |
| `signup` | `TERN_SIGNUP` | `closed` | `open` lets anyone create an account on `/tern/v1/login`. |
| `[features] tracking`, `sharing` | | `true` | Server-side switches; advertised only once implemented. |
| `trusted_proxies` | `TERN_TRUSTED_PROXIES` | `["127.0.0.0/8", "::1"]` | Peers whose `X-Forwarded-For` is used to find the client address for login rate limits. Comma-separated in the env var; empty trusts none. With Docker, add the address the proxy connects from (for example the bridge network, `172.16.0.0/12`). |

## Users and tokens

    tern-backend user add you@example.com --first-name You     # prints a generated password
    tern-backend user add you@example.com --password-stdin < pw.txt
    tern-backend user list | disable | enable | delete --yes <email>
    tern-backend token create <email> [--name laptop]           # prints a device token
    tern-backend token list [--user <email>]
    tern-backend token revoke <id>
    tern-backend migrate
    tern-backend backup /path/to/copy.db                        # VACUUM INTO, safe while running
    tern-backend rotate-server-id                               # after a restore, server stopped

With Docker, run them as `docker exec tern tern-backend user add …`.

Signing in from the desktop at `/tern/v1/login` mints one token per device; revoke it to sign that
device out. Don't share one token between two running clients: a new metadata stream from a
token replaces that token's previous stream for the same account.

## Backups and restores

`backup` writes an owner-only copy and gives it a new server id. To restore, stop the server and
copy the file to `<data_dir>/tern.db` (removing any `tern.db-wal` and `tern.db-shm` left
beside the old one). Clients see the new server id and re-sync their metadata from scratch,
which is what keeps them from skipping writes made after the restore.

If you restore the data dir some other way (a filesystem snapshot, a file-level backup of
`tern.db`), run `tern-backend rotate-server-id` with the server stopped before starting it.

The data dir is created 0700 and `tern.db` 0600, since it holds password and token hashes.

## Reverse proxy

The metadata stream (`/deltas/<aid>/streaming`) is a long-lived chunked response with a 16-byte
heartbeat every 5 s, and clients drop it after 30 s without data. A proxy that buffers
responses breaks it, so turn buffering off.

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

## Licence

`tern-backend` is GPL-3.0-only (`apps/backend/LICENSE`). Running it, for yourself or for other
people over the network, puts no obligation on you. If you pass the binary or an image to
someone else, give them the licence, the third-party notices and the corresponding source
(GPLv3 section 6), for example a link to the tagged source next to the download.
