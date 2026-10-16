# What the web client needs from the hosted engine

Every `TODO(engine)` in `src/`, grouped by engine capability. Regenerate the list with
`grep -rn "TODO(engine)" src`. Paths are relative to `apps/web`.

## Transport and auth

- `src/engine/ws.ts:2`: an HTTP/WebSocket server that speaks the protocol plan's JSON-RPC 2.0 envelope (today the engine only has the Unix socket, S1).
- `src/engine/ws.ts:58`, `src/engine/rpc.ts:80`: a defined auth scheme for the socket. The client sends the session's device token in `hello.token`; the engine must accept that, or define a cookie or subprotocol instead.
- `src/engine/ws.ts:133`: distinguishable failures: "unreachable" versus "auth rejected" (close codes or an HTTP health route).
- `src/engine/ws.ts:141`: reconnect semantics. After a drop the client must reconnect with backoff, re-send `hello` + `subscribe`, and invalidate every query (plan 1.4). The engine side needs nothing new, but the client part is not written yet.
- `src/engine/provider.tsx:22`: `account.state` notifications (phase C) to drive per-account error banners.
- `src/features/auth/session.ts:23`: either one origin serving `/tern/v1/*` and the engine socket with the same device token, or a documented way for the engine to validate Tern server tokens.
- `src/features/auth/session.ts:25`: forward the backend's `identity-refresh-needed` event to web clients so they can call `revalidateSession()`.
- `src/features/auth/session.ts:72`: login sets an httpOnly, Secure, SameSite=Strict cookie, so the token can leave localStorage.
- `src/features/auth/transport.ts:115`: CORS on the server for the web origin (or same-origin hosting).
- `src/features/auth/transport.ts:160`: a JSON login, e.g. `POST /tern/v1/login` with `Accept: application/json` returning the LoginResult or 401. Today the client posts the HTML form and scrapes `#tern-login-result`.

## Read API

- `src/engine/rpc.ts:239`: `accounts.list`, which is not in the protocol plan. The headless engine owns account settings and the web sidebar needs them. It must not return passwords (see Accounts).
- `src/engine/hooks.ts:192`: flag filters on `threads.list` (`starred`, `unread`), or `role: 'starred'` for accounts without a Starred label. Starred views page through the whole account and filter client-side today; the engine silently ignores unknown params, so sending `starred: true` would return every thread.
- `src/engine/model.ts:104`: a sort key on `threads.list`. The engine pages by `lmt` (lastMessageTimestamp) only, so lists can't order by last received or last sent as the desktop's Inbox and Sent do.
- `src/features/mailbox/use-unified-threads.ts:20`: `threads.list` without `accountId` (as `search` allows), so the engine pages unified inboxes instead of the client merging one list per account.
- `src/features/mailbox/list/senders.ts:3`: a message count and per-message senders on Thread JSON (or a thread summary field). Rows estimate both today.
- `src/engine/hooks.ts:387`, `src/engine/hooks.ts:414`: `search` and `contacts.search` without `accountId` (or with several). Both require one, so global search and recipient autocomplete call every account and merge client-side, with no paging.
- `src/features/search/query.ts:172`, `src/engine/mock/search.ts:3`, `src/features/search/SearchView.tsx:225`: search operators. The engine's `search` is plain text (every term quoted for FTS5), so `from:`, `to:`, `subject:`, `in:`, `is:` and `has:` are matched as literal words, and `before:`/`after:` can't be expressed (date filters run client-side, so a date-only query can't be sent). Only the mock understands the grammar.
- `src/engine/cache.ts:218`: calendars, events and contact groups have no views yet. The deltas are ignored until they do.
- `src/engine/mock/engine.ts:342`: `calendar.sync` with real calendar data (the mock has no calendar fixtures).

## Tasks

- `src/engine/mock/apply-task.ts:3`: every task the mock applies locally stands in for ms-tasks. Delete the file when the WebSocket client becomes the default.
- `src/engine/tasks.ts:98`: `ExpungeAllInFolderTask`, `SyncbackCategoryTask` and `DestroyCategoryTask` have no local phase in ms-tasks, so nothing changes until the remote phase and the next folder sync. UI built on them must wait for the Folder/Label deltas, not the `task.queue` result; the mock now behaves the same way.
- `src/engine/mock/apply-task.ts:142`: `GetMessageRFC2822Task`, `EventRSVPTask` and the contact tasks have no mock behaviour.
- `src/engine/mock/engine.ts:314`: `task.cancel` semantics. A real cancel only stops the remote phase, so the local changes stay.
- `src/features/composer/useComposerSession.ts:205`: a delayed send on the engine (`SendDraftTask` with a not-before time) so undo-send survives a closed tab.
- `src/features/composer/send-later-runner.ts:3`: a server-side sender for scheduled drafts. The engine already fires `metadata-expiration` deltas and an open tab sends the draft (as the desktop's send-later plugin does), but nothing sends it while no client is connected.

## Accounts

- `src/engine/mock/engine.ts:381`: phase C `account.add`, which starts syncing the new account.
- `src/engine/mock/engine.ts:397`: phase C `account.test`, which runs the engine's IMAP/SMTP checks.
- `src/features/settings/accounts/actions.ts:30`: `account.test {accountId}` using stored credentials, because a browser should never hold the passwords that `account.test {account}` needs.
- `src/features/settings/accounts/actions.ts:47`: `account.reset`, which streams the resync as deltas. The client refetches everything for now.
- `src/features/settings/accounts/account-draft.ts:108`: the engine assigns the account id on `account.add`. The client mirrors the desktop's `idForAccount` today.
- `src/features/settings/accounts/presets.ts:25`: an OAuth redirect endpoint and token storage for Gmail and Microsoft. Until then those presets use IMAP with an app password.
- `src/features/settings/accounts/presets.ts:110`: `account.discover {email}` (provider table plus MX lookup) to replace the `imap.<domain>` guess.

## Attachments

- `src/engine/client.ts:28`, `src/engine/ws.ts:116`, `src/features/mailbox/reader/AttachmentList.tsx:30`: an authenticated download route, e.g. `GET /files/:accountId/:fileId`, with the socket's auth. `WsEngineClient.fileUrl` returns null until it exists.
- `src/features/composer/useComposerSession.ts:132`: an upload route, e.g. `POST /files/:accountId` returning a File id. Only the File record is saved with the draft today.
- `src/features/composer/draft.ts:193`: when a forward re-attaches the original's File records by id, the engine copies those parts into the new MIME message.

## Integrations

- `src/engine/rpc.ts:243`, `src/engine/mock/engine.ts:430`: params and result shapes for `integrations.translate`, `.grammar` and `.enrich`.
- `src/features/settings/integrations/useIntegrationTest.ts:10`: a result shape for `integrations.test`. The client accepts `{ok, error?}`, and the mock now returns that.
- `src/features/settings/integrations/store.ts:61`: `integrations.configure` so that API keys are stored by the engine (OS keychain, plan section 4). Keys live only in tab memory today.
- `src/features/settings/capabilities.ts:126`: engine-reported Tern server (metadata backend) state, so the server-sourced capabilities (read receipts, link tracking, sharing, metadata sync) can turn on.

## Preferences

- `src/lib/preferences.ts:2`, `src/features/settings/general/GeneralSettings.tsx:16`: per-user preferences (density, reading pane, mark-read delay, remote images, undo-send) stored by the engine or backend.
- `src/features/settings/signatures/store.ts:3`: signatures stored server-side.
- `src/features/composer/templates.ts:3`: templates stored server-side (desktop parity: the Templates folder).
- `src/features/search/recent.ts:2`: recent searches synced with the other preferences.
