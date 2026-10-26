# Tern web

The browser client for Tern. It talks to the Rust engine (`apps/engine`) over its JSON-RPC
2.0 protocol, and builds to static files that a Rust server
will serve; there is no Node runtime in production.

It reaches the engine through `apps/gateway`, which serves this app's `dist/` and relays
JSON-RPC over a WebSocket on the same origin. Without a gateway
the app runs against `MockEngine`, an in-browser implementation of the same methods, live
queries and delta notifications over fixture data (two accounts, about sixty threads). Thread
lists, threads, messages, folders, labels and search results are engine live queries
(`query.subscribe`) kept in the TanStack cache by `src/engine/live.ts`; the web evaluates no
matchers and sorts no lists. Places that need engine
work are marked `// TODO(engine): …` and listed in `TODO-engine.md`.

Stack: Vite, React 19, TypeScript (strict), Tailwind CSS v4, shadcn/ui (Radix), TanStack
Router (file routes) and TanStack Query, Vitest with React Testing Library.

## Running

```sh
npm install
npm run dev        # http://localhost:5173, mock engine
npm run build      # static files in dist/
npm run preview    # serve dist/
npm run typecheck
npm run lint
npm test
```

### Browser run against a real engine

`npm run e2e` builds the app in gateway mode and starts `e2e/stack.py`: the engine harness's
fake IMAP and SMTP servers with two seeded accounts, a `serve` engine on a fresh profile
under `/tmp` and `tern-gateway` serving the build, on ports the system picks. Playwright
(Chromium; `npx playwright install chromium` once) then pairs, reads, searches, triages,
sends with an attachment, unsubscribes, snoozes, signs out and restarts the engine
(`e2e/web.spec.ts`). It needs `target/release/tern-engine` and `target/release/tern-gateway`
(or `TERN_ENGINE_BIN`, `TERN_GATEWAY_BIN`) and is not part of CI. Screenshots land in
`<work>/shots`; `E2E_STACK=<work>/stack.json` reuses a stack started by hand.

### Mock or gateway

| Engine    | Selected by                                          | Client                                   |
| --------- | ---------------------------------------------------- | ---------------------------------------- |
| `mock`    | default                                              | `MockEngine`, in the browser, no network |
| `gateway` | `VITE_TERN_ENGINE=gateway` (the gateway's own build) | `WsEngineClient` on `/gateway/v1/ws`     |

In dev builds `localStorage.setItem('tern.engine', 'gateway' | 'mock')` overrides the build
setting. Against a local gateway, `npm run dev` proxies `/gateway` (HTTP and WebSocket) to
`TERN_GATEWAY_URL` (default `http://127.0.0.1:5180`) without rewriting Host or Origin, so
start the gateway with `--allow-origin http://localhost:5173`:

```sh
tern-gateway serve --allow-origin http://localhost:5173
VITE_TERN_ENGINE=gateway npm run dev
tern-gateway pair            # prints a code and a /pair#<code> link; use the dev origin
```

The browser pairs with the gateway (`/pair`) and gets an httpOnly cookie; it never holds a
token. In mock mode the browser starts paired and any 12-character base32 code pairs again.

Both clients implement `EngineClient` (`src/engine/client.ts`), so views and hooks don't
change. `WsEngineClient` checks `GET /gateway/v1/session` before each connect, reconnects with
backoff and re-reads every query afterwards. In the mock, `sync.wake` (or a timer
every 90 s in dev) delivers a scripted new thread, a few older messages arrive without bodies
to exercise `bodies.fetch`, and one spam message carries a script to check the sandbox.

## Layout

```
src/engine/           wire types (from @tern/client), JSON-RPC types, EngineClient, MockEngine,
                      WsEngineClient, provider, TanStack Query hooks, live.ts (live queries in
                      the query cache), feed.ts (the deltas without a live query), task builders
src/components/ui/    shadcn components (generated; edit with care)
src/components/       AppShell, MailPanes, email/ (sandboxed body rendering), shared bits
src/features/<name>/  mailbox, composer, search, settings, auth
src/routes/           TanStack file routes; routeTree.gen.ts is generated
src/lib/              shortcuts registry, preferences, formatting
DESIGN.md             design direction, tokens, keyboard conventions
```

Email HTML is rendered in an iframe with `sandbox="allow-same-origin"` (never
`allow-scripts`), a CSP that blocks remote content unless the reader loads images, and
link clicks handled by the app (`noopener,noreferrer`). Use `MessageBody` from
`src/components/email`; don't render message HTML any other way.

## TODO(engine)

`TODO-engine.md` lists every marker with where its work lands (a desktop client stage,
the gateway, or web-only).
