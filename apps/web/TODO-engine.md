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

Done (markers removed): the sidebar's unread and total counts
come from `counts.get` and `ThreadCounts` deltas, not from refetching `folders.list` and
`labels.list` on every Thread delta; the agenda shows the occurrences `events.expand`
expands, recurring events included, in the browser's zone; the reader shows the engine's
message `warnings` (phishing: replies that go to another domain) and leaves out `hidden`
messages.

Done (marker removed): thread lists are live queries with
`include: ["summary"]`, so a row's message count, senders, snippet and attachment icon come
from the engine's thread summary (`src/features/mailbox/list/senders.ts`).

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

Done with DCP 4 (marker removed): the engine sends due send-later drafts itself, so a
scheduled send goes out with no tab open.

Done: the engine builds new drafts, replies and forwards
(`draft.create`, not saved: addresses, subject, quote, signature, automatic Cc/Bcc, a
forward's files), and the web's reply and quote builders went to the mock. Send saves the
draft, asks `draft.validate` (errors stop it, warnings such as a blank subject ask first)
and calls `draft.send`; with an undo delay the engine writes send-later `{expiration,
isUndoSend}` by its own clock and its executor sends, so a send survives the tab closing,
and Undo is `draft.cancelSend`. The templates menu lists and inserts the engine's templates
(`templates.list|get`, the desktop's files); saving and deleting them stays on the desktop
while `templates.save|delete` are local-only, and the old per-browser templates are gone.
Scheduling a send still writes the send-later row itself (`draft.schedule`).

## Accounts

`account.add`, `account.test` and `account.reset` exist and take a device paired with the
admin scope; `account.discover` is open to every device.

- `src/features/settings/accounts/actions.ts:30`: `account.test {accountId}` with stored
  credentials; the engine has no such form.
- `src/features/settings/accounts/account-draft.ts:138`: `account.add` derives the id when
  the account has none; the form should stop making one.
- `src/features/settings/accounts/presets.ts:25`: OAuth from the web. Deferred: a fixed
  redirect page for `account.oauth.*`; app passwords until then.
- `src/features/settings/accounts/presets.ts:111`: the account form on `account.discover`.

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

Done with DCP 5 (markers removed): the Airpost server's features (read receipts, link tracking,
sharing, sync across devices) come from the engine's `capabilities.server` in `hello` and
`capabilities.changed` (`EngineFeature.Capabilities`); without it there is no server.
`integrations.test` sends the engine's shape (`{config: {provider, url?, apiKey?,
username?}}`, `pdl` as `peopledatalabs`), shows "Connected to <host>" from its `TestReport`
and the reason from a `-32010`; required fields are checked first, since the engine refuses
a config without them as invalid params. `integrations.*` are local-only, so only an admin
device gets past the gateway; translate, grammar and enrich have no web view yet.

- `src/features/settings/integrations/store.ts:61`: keys stored by the engine with
  `secrets.set` (admin scope); per request (DCP 5) until then. Relaying `integrations.*` to
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
