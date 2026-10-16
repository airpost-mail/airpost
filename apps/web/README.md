# Tern web

The browser client for Tern. It talks to the Rust engine (`apps/engine`) over its JSON-RPC
2.0 protocol (`docs/engine-protocol-plan.md`), and builds to static files that a Rust server
will serve; there is no Node runtime in production.

The hosted engine does not exist yet. Until it does, the app runs against `MockEngine`, an
in-browser implementation of the same methods and delta notifications over fixture data
(two accounts, about sixty threads). Places that need real engine behaviour are marked
`// TODO(engine): …`.

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

### Mock or real engine

| `VITE_ENGINE_URL`                  | Client                                    |
| ---------------------------------- | ----------------------------------------- |
| unset                              | `MockEngine`, in the browser, no network  |
| `ws://host:port/path` or `wss://…` | `WsEngineClient`, JSON-RPC over WebSocket |

```sh
VITE_ENGINE_URL=ws://localhost:7420/rpc npm run dev
```

Both implement `EngineClient` (`src/engine/client.ts`), so views and hooks don't change.
In the mock, `sync.wake` (or a timer every 90 s in dev) delivers a scripted new thread, a few
older messages arrive without bodies to exercise `bodies.fetch`, and one spam message
carries a script to check the sandbox.

## Layout

```
src/engine/           wire types, JSON-RPC types, EngineClient, MockEngine, WsEngineClient,
                      provider, TanStack Query hooks, delta -> cache logic, task builders
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

What the web client needs from the engine that the protocol plan doesn't cover yet:

- An HTTP/WebSocket transport. The plan (S1) has a Unix socket only; the web client
  expects JSON-RPC 2.0 over WebSocket at `VITE_ENGINE_URL`.
- Auth: a token in `hello`, a cookie from the backend's login, or a subprotocol. Wire it in
  `src/engine/ws.ts` and `src/features/auth`.
- Reconnection: backoff, re-`hello`, re-`subscribe`, then invalidate the query cache.
- `accounts.list`: full account objects (the plan's `hello`/`engine.status` only return
  `{accountId, email, state}`).
- `categories.counts`: unread/total per folder and label (the desktop reads ThreadCounts
  from SQLite directly).
- `threads.list` with `threadIds`, for loading one thread by id.
- Attachment downloads over HTTP (`EngineClient.fileUrl`).
- Confirm the `threads.list` / `search` paging shape (`{items, next: {ts, id}}`) and the sort
  key (the mock sorts by `lmrt`, the plan says last-message timestamp).
- Phase C `account.*` methods and `account.state` notifications.
- `integrations.*` params and results.
- Calendar, events and contact groups (no fixtures or views yet).

## Licence

GPL-3.0-only. See `LICENSE` and the repository root `LICENSE.md`. Fonts are OFL and bundled.
