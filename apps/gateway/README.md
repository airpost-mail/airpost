# tern-gateway

Authenticated access to a Tern engine from a browser. It serves the built web client
(`apps/web`) and relays its JSON-RPC WebSocket to the engine's Unix socket, as the engine's
own user.

Until the engine can run headless, the gateway reaches the
engine a running desktop started (`serve` mode, `<profile>/run/engine.json`). When the
desktop quits, the engine stops and browsers show "The engine isn't running".

## Run it locally

    cargo build --release -p tern-gateway
    (cd apps/web && VITE_TERN_ENGINE=gateway npm run build)
    target/release/tern-gateway serve --web-root apps/web/dist
    target/release/tern-gateway pair            # prints a code and a /pair#… link

Open the link on this machine (default `http://127.0.0.1:5180`; with `--bind 127.0.0.1:0`
the system picks the port, which the link and `<profile>/gateway/gateway.json` name). Without `--config-dir` (or
`TERN_CONFIG_DIR`) the gateway uses the desktop's profile, like `tern`.

Web client development: run the gateway with `--allow-origin http://localhost:5173` and
`npm run dev` in `apps/web` with `VITE_TERN_ENGINE=gateway`; Vite proxies `/gateway`.

## Devices

    tern-gateway pair [--admin] [--name <device>] [--ttl 10m]
    tern-gateway devices list [--json]
    tern-gateway devices revoke <id>        # closes that device's open sessions
    tern-gateway devices rename <id> <name>

A code works once, for at most an hour. A paired browser holds an httpOnly cookie; the
gateway stores only hashes, in `<profile>/gateway/devices.json`. `--admin` lets the device
use account, secret and engine methods once the engine has them; other devices get mail
reads, triage and drafts. Devices unused for 180 days are dropped, admin devices after 30.

## Remote access

The gateway binds `127.0.0.1` and speaks plain HTTP. For other machines put a TLS proxy in
front of it, or use a private network with its own certificate (`tailscale cert`), and set
the public URL:

    TERN_GATEWAY_BIND=127.0.0.1:5180 TERN_GATEWAY_PUBLIC_URL=https://tern.example.com \
        tern-gateway serve --web-root /usr/share/tern-gateway/web

Caddy forwards WebSocket upgrades by default:

    tern.example.com {
        reverse_proxy 127.0.0.1:5180
    }

nginx needs the upgrade headers and the original `Host`, which the gateway checks:

    location / {
        proxy_pass http://127.0.0.1:5180;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_read_timeout 1h;
    }

The gateway pings every WebSocket every 30 s and closes one that has sent nothing for 45 s
(a sleeping laptop, a dropped network), which also ends that tab's engine connection and
any AI stream it started. The traffic keeps idle WebSockets inside a proxy's read timeout;
nginx's default of 60 s is enough, so `proxy_read_timeout` above is only a margin.

A plain-http public URL on another host is refused unless `--insecure-public-url` is given;
the cookie then travels unencrypted.

| Env | Default | |
|---|---|---|
| `TERN_CONFIG_DIR` | the desktop's profile | the engine's profile; gateway state in `<dir>/gateway/` |
| `TERN_GATEWAY_BIND` | `127.0.0.1:5180` | |
| `TERN_GATEWAY_PUBLIC_URL` | the bind address, if loopback | required otherwise |
| `TERN_GATEWAY_INSECURE_PUBLIC_URL` | false | allow plain http off loopback |
| `TERN_GATEWAY_WEB_ROOT` | none | the built `apps/web/dist` |
| `TERN_GATEWAY_ALLOW_ORIGINS` | none | extra origins, comma-separated (a Vite dev server) |
| `TERN_TRUSTED_PROXIES` | `127.0.0.0/8,::1` | whose `X-Forwarded-For` counts for rate limits |

Health: `GET /healthz`, or `tern-gateway healthcheck` where there is no curl. It reports the
gateway, not the engine; `/gateway/v1/session` reports the engine to a paired browser.

Attachments: `GET /gateway/v1/files/<accountId>/<fileId>` downloads one through the
engine's `files.get` (which fetches it from the mail server first when needed), always as an
attachment in a CSP sandbox, with `?inline=1` honoured only for PNG, JPEG, GIF and WebP
recognised from the bytes. `POST /gateway/v1/files/<accountId>?name=<name>` takes up to
25 MiB, streamed to `<dir>/gateway/incoming/`, hands it to `files.put` and deletes it; a
larger one gets 413 (a chunked body is read to its end first, for up to 5 s, so the client
sees the answer). A proxy in front must allow bodies that large (nginx:
`client_max_body_size 26m;`).
