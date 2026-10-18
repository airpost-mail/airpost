# What the web client still needs from the engine

Every `TODO(engine)` in `src/`, grouped as in `docs/web-gateway-plan.md` section 4, with where
the work lands: **DCP n** is `docs/desktop-client-plan.md` stage n, **GW** the gateway, **W5**
etc. the plan's steps (section 7). Regenerate the list with `grep -rn "TODO(engine)" src`.
Paths are relative to `apps/web`.

Done in step W1 (markers removed): the WebSocket transport and its auth (pairing cookie on
the upgrade, no `hello.token`), unreachable versus unpaired (`/gateway/v1/session` and close
codes), the reconnect contract, `account.state` driving the account state, the session
model (pairing instead of backend sign-in), `account.list` in place of `accounts.list`. CORS
and the backend's JSON login are not needed in v1 (plan G3, G8). Task cancel semantics and
the expunge/category tasks without a local phase are documented, not engine work.

## Read API

Done in step W6 (DCP 7, markers removed): thread lists read with `query` behind
`hello.features` (`EngineFeature.Query`), so the engine filters Starred and unread (`where`),
sorts by last received or, in Sent, last sent (`order`), and pages the unified views itself
(`accounts: null`); contact autocomplete across accounts in one `query` on Contact; calendar
and event reads with a read-only agenda, and calendar fixtures in the mock. Engines without
`query` keep the old paths (per-account merge, client-side flag filters).

- `src/features/mailbox/list/senders.ts:3`: message count and per-message senders on Thread
  JSON. No DCP stage yet (a full Message `query` per list page is the wrong cost).
- `src/engine/hooks.ts:522`: `search` across accounts. DCP 9.
- `src/features/search/query.ts:172`, `src/engine/mock/search.ts:3`,
  `src/features/search/SearchView.tsx:225`: search operators and dates. DCP 9.
- `src/features/calendar/CalendarView.tsx:3`: creating, editing and answering events, and
  recurrence expansion. DCP 18.

## Tasks

- `src/engine/mock/apply-task.ts:147`: RSVP and contact tasks in the mock. DCP 18.
- `src/features/composer/useComposerSession.ts:246`: undo-send surviving a closed tab. DCP 4
  (send-later metadata fired by the executor), then DCP 13 (`draft.send {delayMs}`).
- `src/features/composer/send-later-runner.ts:3`: sending with no tab open. DCP 4 executor
  (gate the runner on `expiration-executor`); with no desktop running, DCP 11.

## Accounts

- `src/engine/mock/engine.ts:427`: `account.add` from the web. DCP 11/12, admin scope.
- `src/engine/mock/engine.ts:444`: `account.test` on the socket. DCP 12, admin scope.
- `src/features/settings/accounts/actions.ts:30`: `account.test {accountId}` with stored
  credentials. DCP 12.
- `src/features/settings/accounts/actions.ts:47`: `account.reset` on the socket. DCP 12, admin
  scope; the provider already refetches on `account.state` starting → running.
- `src/features/settings/accounts/account-draft.ts:108`: the engine assigns the account id.
  DCP 11/12.
- `src/features/settings/accounts/presets.ts:25`: OAuth from the web. Deferred: a fixed
  redirect page plus DCP 12's `account.oauth.*`; app passwords until then.
- `src/features/settings/accounts/presets.ts:111`: `account.discover`. DCP 12.

## Attachments

Done in step W5 (DCP 16, markers removed): downloads link to the gateway's file route when
the engine announces `files` (`EngineFeature.Files`); `cid:` images load from it with
`?inline=1`, which the reader frame's CSP allows (`img-src` names the file route); the
composer uploads each picked file at once (`POST /gateway/v1/files/:accountId`, `files.put`),
again when From moves to another account, and saves and sends only uploaded ones; before SendDraftTask (and before scheduling a send) every
attachment, a forwarded original's included, is fetched with `HEAD` on its download route,
which makes the engine download it. GetMessageRFC2822Task's file-id mode is in the types
and the mock; nothing in the web asks for a raw message yet.

## Integrations

- `src/engine/rpc.ts:349`, `src/engine/mock/engine.ts:514`: translate, grammar and enrich
  shapes. DCP 5.
- `src/features/settings/integrations/useIntegrationTest.ts:10`: `integrations.test` result.
  DCP 5.
- `src/features/settings/integrations/store.ts:61`: keys stored by the engine. DCP 11/12
  `secrets.set`, admin scope; per request (DCP 5) until then.
- `src/features/settings/capabilities.ts:126`: Tern server state. DCP 5 `capabilities`.

## Preferences

- `src/features/settings/general/GeneralSettings.tsx:16`: UI preferences stay per browser in
  v1; `core.sending.undoSend` moves with DCP 3.
- `src/features/settings/signatures/store.ts:3`: signatures. DCP 3.
- `src/features/composer/templates.ts:3`: templates. DCP 3.
