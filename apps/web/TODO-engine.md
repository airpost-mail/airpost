# What the web client still needs from the engine

Every `TODO(engine)` in `src/`, grouped by area, with where the work lands: **DCP n** is
desktop client stage n (engine work shared with the desktop), **GW** the gateway, **W5** etc.
the web client's own steps. Regenerate the list with `grep -rn "TODO(engine)" src`.
Paths are relative to `apps/web`.

Done in step W1 (markers removed): the WebSocket transport and its auth (pairing cookie on
the upgrade, no `hello.token`), unreachable versus unpaired (`/gateway/v1/session` and close
codes), the reconnect contract, `account.state` driving the account state, the session
model (pairing instead of backend sign-in), `account.list` in place of `accounts.list`. CORS
and the backend's JSON login are not needed in v1. Task cancel semantics and
the expunge/category tasks without a local phase are documented, not engine work.

## Read API

Done in step W6 (DCP 7, markers removed): thread lists read with `query` behind
`hello.features` (`EngineFeature.Query`), so the engine filters Starred and unread (`where`),
sorts by last received or, in Sent, last sent (`order`), and pages the unified views itself
(`accounts: null`); contact autocomplete across accounts in one `query` on Contact; calendar
and event reads with a read-only agenda, and calendar fixtures in the mock. Engines without
`query` keep the old paths (per-account merge, client-side flag filters).

Done in step W7 (DCP 9, markers removed): with `EngineFeature.SearchGrammar` the search page
and the palette send the query as typed (`search {grammar: true, tz, now}`), so the engine
reads `from:`, `in:`, `is:`, `has:attachment`, `AND`/`OR`/`NOT`, parentheses and
`before:`/`after:` in the browser's zone, and searches every account in one call (no
`accountId`, one row past the limit for "more"). A query the engine can't read shows its
message. Engines without the feature keep the old path (plain words per account, dates
applied by the client, "Almost there" for dates alone). The mock implements the engine's
grammar and date subset (`src/engine/mock/search.ts`, tested against the engine's cases);
`MockEngine({features: ['query']})` stands in for an engine without it.

- `src/features/mailbox/list/senders.ts:3`: message count and per-message senders on Thread
  JSON. No DCP stage yet (a full Message `query` per list page is the wrong cost).
- `src/features/calendar/CalendarView.tsx:3`: creating, editing and answering events, and
  recurrence expansion. DCP 18.

## Tasks

Done with DCP 15 (no markers existed): snooze and unsubscribe go through the engine when it
announces `snooze` and `unsubscribe` (`EngineFeature.Snooze`, `EngineFeature.Unsubscribe`).
The reader's snooze menu (`z`, the desktop's presets or a picked time, "Unsnooze now" in
Snoozed) calls `threads.snooze` / `threads.unsnooze`, which queue the move and the
`thread-snooze` metadata and leave the return to the engine's expiration executor; rows move
on the deltas, not optimistically. The Unsubscribe button shows on messages whose
List-Unsubscribe header has an http(s) or mailto URI and calls `messages.unsubscribe`: the
engine makes the one-click POST or sends the mailto message itself, and a web link comes back
for a toast's Open button (a click, so popup blockers allow it). Links found only in the body
aren't looked for, as on the desktop. Without the features neither control shows. The mock
does both (`src/engine/mock/mail-actions.ts`), including the return when a snooze comes due.

Done with DCP 4 (marker removed): with `expiration-executor` (`EngineFeature.ExpirationExecutor`)
the engine sends due send-later drafts itself and the web's runner stays out of the way, so
a scheduled send goes out with no tab open (and, until DCP 11, while the desktop runs the
engine). Engines without the feature keep the runner. Undo send uses the same path, as the
desktop does: Send saves the draft with send-later `{expiration: now + delay, isUndoSend:
true}` and follows the executor's SendDraftTask for the toast, Undo clears it with
`{expiration: null, isUndoSend: true}`, so a send survives the tab closing inside the delay.
Without the feature the delay stays a timer in the tab. The mock runs the executor's
send-later half when told to announce the feature.

- `src/engine/mock/apply-task.ts:147`: RSVP and contact tasks in the mock. DCP 18.

## Accounts

- `src/engine/mock/engine.ts:620`: `account.add` from the web. DCP 11/12, admin scope.
- `src/engine/mock/engine.ts:637`: `account.test` on the socket. DCP 12, admin scope.
- `src/features/settings/accounts/actions.ts:30`: `account.test {accountId}` with stored
  credentials. DCP 12.
- `src/features/settings/accounts/actions.ts:47`: `account.reset` on the socket. DCP 12, admin
  scope; the provider already refetches on `account.state` starting → running.
- `src/features/settings/accounts/account-draft.ts:138`: the engine assigns the account id.
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

Done with DCP 5 (markers removed): the Tern server's features (read receipts, link tracking,
sharing, sync across devices) come from the engine's `capabilities.server` in `hello` and
`capabilities.changed` (`EngineFeature.Capabilities`); without it there is no server.
`integrations.test` sends the engine's shape (`{config: {provider, url?, apiKey?,
username?}}`, `pdl` as `peopledatalabs`), shows "Connected to <host>" from its `TestReport`
and the reason from a `-32010`; required fields are checked first, since the engine refuses
a config without them as invalid params. `integrations.*` are local-only, so only an admin
device gets past the gateway; translate, grammar and enrich have no web view yet.

- `src/features/settings/integrations/store.ts:61`: keys stored by the engine. DCP 11/12
  `secrets.set`, admin scope; per request (DCP 5) until then. Relaying `integrations.*` to
  devices without admin needs the engine to use its own saved config and keys (DCP 3.6).

## Preferences

Done with DCP 3 (marker removed): with `settings` (`EngineFeature.Settings`) the undo-send
delay is the engine's `core.sending.undoSend`, read on connect (`settings.get`), followed
through `settings.changed` and written with `settings.set`, so the desktop and every paired
browser share it (`src/engine/settings.ts`, `src/lib/undo-send.ts`); this browser's copy is
the fallback for an engine without settings or without a value. Other UI preferences stay
per browser in v1. The mock keeps a settings store and refuses the keys a
device without admin can't set, as the engine does.

- `src/features/settings/signatures/store.ts:3`: signatures. The engine has `signatures`, but
  a device without admin can't set them until the desktop treats web-written signature HTML
  as untrusted and the key joins `RELAYED_SETTINGS_KEYS`.
- `src/features/composer/templates.ts:3`: templates. Same refusal for `templates`, and the
  desktop keeps its templates as files rather than in the store.
