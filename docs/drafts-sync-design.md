# Server-synced drafts (design)

Status: engine side implemented 2026-10-21 (3.1-3.3, 3.5, the 3.6 scenarios); see
"Implementation" at the end. The per-account setting is `save_drafts_to_server`, default on;
the harness and the A/B runs set it off. Backlog item F4 in `docs/fix-backlog.md`. C++ paths
relative to upstream Mailspring-Sync, desktop paths relative to `apps/desktop/`.

## 1. Today (C++ and Rust behave the same)

**Saving (`SyncbackDraftTask`).**
- Local phase `TaskProcessor.cpp:1320-1357` / `crates/tern-tasks/src/drafts.rs:130-168`.
  `inflateClientDraftJSON` (`TaskProcessor.cpp:1043-1084`) merges client JSON over the row's
  `_data`, so engine bookkeeping in `_data` survives saves.
- New draft id: `draft_id` = base58(sha256(`aid:hMsgId`)) (`MailUtils.cpp:572-578`).
- `ensureDraftPlacement` (`:1359-1369`) adds a UID-0 placement in Drafts; on Gmail, Drafts is a
  Label, so it falls back to All Mail at UID 0. UID-0 draft rows are exempt from cleanup
  (`MailStore.cpp:1224-1229`, `crates/tern-store/src/placement.rs:606-611`).
- Remote phase: `// right now we don't syncback drafts` (`TaskProcessor.cpp:922-923`), since 2017.

**Why it was never built.** Mailspring #1726 (2019): draft sync existed in the Nylas cloud era,
"it took a lot of work to get the synchronization right", never re-done. Requests: #1417, forum
topics 2874 and 9254.

**Server drafts from other clients** are ingested as normal messages with the header-hash id
(`MailUtils.cpp:620`, `crates/tern-sync/src/mail_processor.rs:51`); `draft` comes from `\Draft` or
the drafts-role folder (`MailStore.cpp:58-77`, `placement.rs:101-125`). On Gmail they arrive via
All Mail with `\Draft` in X-GM-LABELS. The Drafts view is every Message with `draft = true`
(`draft-list-store.ts:53`).

**Editing one loses data.** `DraftEditingSession` loads by `{headerMessageId, draft: true}`
(`draft-editing-session.ts:223`); the first edit updates that row in place, keeping the server
UID, so the server copy is silently stale. A later webmail edit creates a new UID and hash id
(a second draft); the old UID vanishes, the row with the local edits is orphaned and swept.

**Deleting (`DestroyDraftTask`)**: `deleted-…` stub takes the placements with a pending move to
Trash (`TaskProcessor.cpp:1379-1444`, `drafts.rs:176-231`); covered by `synced-draft-destroy.yaml`,
which asserts no APPEND.

**After send**: `_removeMessageCopiesResilient` deletes copies in the draft's known placements
(`TaskProcessor.cpp:2275-2278`, `send.rs:239-241`); it would miss a just-APPENDed copy.

**Client timing**: `DraftChangeSet` commits after 10 s idle (`draft-change-set.ts:14`); replies
create a `pristine: true` row immediately (`draft-factory.ts:70`); Message-IDs are
`UUID@getmailspring.com` (`draft-factory.ts:66`).

## 2. Other clients

- **Thunderbird**: APPEND a new copy, then delete the previous one by stored folder+UID
  (`nsMsgCompose.cpp` `RemoveCurrentDraftMessage`); Message-ID generated once and reused;
  autosave every 5 min; no conflict detection.
- **Apple Mail** (unverified): APPEND then delete; stable `X-Universally-Unique-Identifier`.
- **Gmail IMAP**: APPEND to `[Gmail]/Drafts` creates a Gmail draft threaded by References and
  subject; APPENDUID refers to the Drafts view, not All Mail; `APPENDLIMIT=35651584`. Google's
  docs show the label as `\Drafts`, the engine checks `\Draft` (risk 1). Unknown whether Gmail
  web keeps the Message-ID when editing an IMAP draft.
- **Exchange/O365**: UIDPLUS and MOVE, no CATENATE; Outlook drafts sometimes lack `\Draft`.
- **Standards**: RFC 3501 §2.3.2, RFC 6154, RFC 4315, RFC 4469 (CATENATE, later optimisation).

## 3. Design

### 3.1 Identity
A copy that counts as a draft (placement `draft` set: `\Draft` flag or label and not in
Trash/Spam, or in the drafts-role folder) and has a Message-ID gets `id = draft_id(aid, Message-ID)`.
`id_for_message` takes folder role and attributes; the heavy-scan id check (`worker.rs:333`)
then agrees.
- Our APPENDed copy maps onto the existing local row; all copies of one draft (old/new, crash
  leftovers, other devices) collapse onto one row with several placements; webmail drafts get
  the id the client would compute.
- No Message-ID: keep the hash id; re-key to `draft_id` on first push (client sees remove+add;
  the composer is keyed by `headerMessageId`).
- Sent copies (no `\Draft`) keep the hash id.

### 3.2 Push
Local phase unchanged plus `_data._dsync = {rev, pushedRev, base:{folderId, uid, gmMsgId},
savedAt, fails, nextAt}`, `rev += 1` per save. The task completes immediately; a
`push_dirty_drafts()` step in the foreground loop (`foreground.rs:180`) does the work, so a large
APPEND never delays a send:
- candidates: `rev > pushedRev`, not `pristine`, quiet for ~20 s (~120 s with >1 MB attachments),
  no queued SendDraft/DestroyDraft; IDLE is woken at the next deadline.
- per push:
  1. Build MIME with the send builder (factor `OutgoingMessage` out of `send.rs:211-232`): keep
     Bcc, `Message-ID: <hMsgId>`, In-Reply-To/References as send, `X-Tern-Draft: 1`. Fetch any
     missing attachment of a server-originated draft first.
  2. APPEND `\Draft \Seen` to the drafts-role folder (Gmail: drafts label path). Over APPENDLIMIT:
     stay local, log.
  3. Resolve the UID now: APPENDUID, else (Gmail, no UIDPLUS) `find_uids_of_recent_header_message_id`
     as send does. Record the placement with the flags the server will report; it replaces UID 0.
  4. Then delete `base` and any older copies with `remove_messages_resilient`. Never zero copies;
     a crash leaves two, which collapse and are cleaned on the next push.
  5. `pushedRev = rev`, `base = new`; on failure back off via `fails`/`nextAt`.

### 3.3 Incoming copies and conflicts
A draft copy at a UID above `base` is another device's revision.
- Newer remote, nothing unpushed: adopt (re-read envelope and files, drop `MessageBody` so it is
  refetched, emit delta with `body`). Open composers take it only in unedited fields
  (`draft-editing-session.ts:499`).
- Newer remote, local unpushed: local wins on the next push; the foreign copy goes to Trash
  without expunge; log `draft-conflict`.
- Remote delete: clean -> orphan sweep removes the row; dirty -> next save recreates it under the
  same id and pushes again.
- `DraftTombstone(accountId, headerMessageId, since)` table (Rust-owned migration): a
  draft copy matching a tombstone is deleted on the server instead of ingested, so sent or
  destroyed drafts don't come back.

### 3.4 Client
Works without changes. Add a per-account "Save drafts to server" setting passed in account
settings (default on after rollout); optionally a non-Mailspring Message-ID domain. Plugin
metadata stays local under D3.

### 3.5 Existing local drafts
On enable: mark UID-0 drafts `rev=1, pushedRev=0`, push non-pristine ones with body or
recipients, rate-limited (includes `import-legacy` drafts). Re-key rows whose id isn't
`draft_id(aid, hMsgId)` at startup. Server drafts with hash ids are re-keyed by the next heavy
scan.

### 3.6 Scenarios (Rust-only, gate on)
`draft-push-basic` (dovecot, cyrus), `draft-push-replace`, `draft-push-coalesce`,
`draft-push-pristine`, `draft-push-no-uidplus` and `draft-push-courier`, `draft-push-gmail`,
`draft-remote-edit-adopt`, `draft-conflict-local-wins`, `draft-remote-delete`,
`draft-send-after-push` and `draft-send-before-scan`, `draft-destroy-after-push`,
`draft-attachments` (incl. over APPENDLIMIT), `import-legacy-drafts-pushed`.

### 3.7 Parity contract
Diverges from C++ in a remote phase, in message identity and schema. Gate off by default so
`ab.py` and existing gates (`synced-draft-destroy`, `send-draft`, `sent-copy-*`, `import-legacy-*`)
still compare like with like.

## 4. Risks
1. Gmail draft detection (`\Draft` vs `\Drafts`): record a Gmail transcript with
   `test/tools/record_personality.py` before building.
2. Other clients rewriting Message-ID on edit: degrades to delete+new draft, not lost edits.
3. Every push re-uploads full MIME: quiet window, APPENDLIMIT; CATENATE later.
4. Send/delete races: synchronous UID resolution and tombstones are required.
5. Heavy-scan resurrection of a draft with a pending deletion stub (`worker.rs:333`): add a scenario.
6. Privacy: drafts visible on the server; covered by the per-account setting.
7. Gmail thread pollution by reply drafts: pristine skip and quiet window; expunge old copies.

## 5. Size
Identity and re-keying ~300 lines; push scheduler, builder refactor, APPEND/UID, Gmail ~700;
adoption ~300; tombstones ~200; attachment prefetch ~200; client ~50; scenarios and fakeimap
~800. About 2.5–3.5k lines, 2–3 weeks.

## 6. Implementation (2026-10-21)

The engine reads `save_drafts_to_server` from the account settings. The desktop has a per-account switch in Preferences > Accounts (an `account.update` restarts the account, so it applies live), and an open composer takes a remote revision only in fields the user hasn't edited since the last save; a re-key keeps the session.
Where it differs from the design above:

- Setting: `save_drafts_to_server` in the account's `settings`, default on
  (`Account::save_drafts_to_server`). Off, the engine behaves as the C++ engine (header-hash
  ids, no pushes, no tombstones); `apps/engine/test/harness/engine.py` turns it off for every
  scenario unless one sets `account: {extra_settings: {save_drafts_to_server: true}}`.
- Identity (3.1): `tern_sync::drafts` and `MailProcessor::insert_fallback_to_update_message`.
  A draft row of that Message-ID under another id is re-keyed onto the draft id when its copy
  is ingested, and before its first push (`Store::rekey_message`: unpersist of the old id,
  persist of the new). Saves and sends that still name the old id are pointed at the row by
  `TaskProcessor::find_client_draft`. Gmail's label is taken as `\Draft` or `\Drafts`.
- Push (3.2): `crates/tern-tasks/src/draft_sync.rs`, run by the foreground after its tasks
  (`RemoteTaskRunner::sync_drafts`), which wakes at the next due push. `_dsync` is
  `{rev, pushedRev, base: {f, u, d}, savedAt, fails, nextAt, held}`; `base.d` is the pushed
  copy's Date, which tells our copy found at a new UID (UIDVALIDITY reset) from another
  device's revision. Quiet window `TERN_DRAFT_PUSH_QUIET` overrides 20 s / 120 s. Five pushes
  per iteration. Backoff 30 s doubling to 1 h. A revision over APPENDLIMIT, or one that can't be
  built (no From, unreadable attachment), is `held` until the next save. The MIME is the send
  builder's (`TaskProcessor::outgoing_message`) with `X-Tern-Draft: 1`; missing attachment data
  of a server draft is taken from its server copy.
- Incoming (3.3): `reconcile_draft_copies`, before the pushes. The background pass interrupts
  the foreground when a scan left such a copy. An adopted revision updates subject, date,
  From/To/Cc/Bcc/Reply-To, In-Reply-To, and body and files through `retrieved_message_body`
  (the delta carries `body`). Older copies left by an adoption go at the next push.
- Tombstones: migration 14, `DraftTombstone`. Added after SMTP accepts a send and on
  DestroyDraftTask's local phase, lifted by a save of that Message-ID (undo send), expired
  after 30 days, cleared by `reset`. Scans skip a tombstoned draft copy and queue it on
  `ForegroundControl`, which deletes it.
- Existing local drafts (3.5): drafts without `_dsync` and without a server copy are marked
  once (`rev = 1` with a body or recipients and not pristine, else `rev = 0`) and pushed
  through the normal path, which re-keys them. There is no separate startup re-key pass.
  Their first push (`savedAt` 0) waits until Sent (All Mail on Gmail) has been walked to UID
  1 once; the background interrupts the foreground when it has.
- Drafts of sent messages (`remove_sent_drafts`, first in the foreground's draft step): a
  draft whose Message-ID a non-draft message in Sent (or labelled Sent) also has is a
  leftover of a send stopped before its draft went (fix-backlog F1 item 2, mostly C++
  drafts brought by Import from Mailspring). It is never pushed: the row goes (unpersist
  delta), its Message-ID is tombstoned, and the draft copies it was pushed as (its own
  placements, `\Draft` or in Drafts) are deleted; the Sent copy is another row and is not
  touched. Skipped while a queued SendDraftTask or DestroyDraftTask names the draft
  (checked inside the write transaction) and when a failed SendDraftTask does (a multisend
  that failed part way). Found on a real Gmail account, 2026-10-21: the first push sent up
  two such drafts, which Gmail showed inside the sent threads (`draft-sent-leftover`,
  `import-legacy-sent-leftover`). A leftover whose Sent copy is not in the local database
  is not recognised.
- Drafts in no thread (`thread_unthreaded_drafts`, last in the draft step;
  `Store::thread_unthreaded_draft`): the desktop shows a draft in the thread list (the
  pencil, the thread's Drafts category) only through its thread, and imported drafts,
  new messages and some synced drafts have an empty `threadId`. Each gets the thread of its
  server copy's X-GM-THRID on Gmail, else the thread of the message it replies to or
  forwards (an unthreaded draft it answers is threaded first), else one that references
  either Message-ID, else a thread of its own (`t:<draft id>`, recording the Message-IDs
  so the answered message and the sent copy join it). The row is saved into the thread as
  a desktop reply draft is (`an_unthreaded_reply_draft_threaded_by_the_engine_matches_the_desktop_reply_draft`).
  On Gmail a draft with no server copy waits for its push unless the message it answers is
  here. A save that sends an empty `threadId` keeps the stored one. Runs every foreground
  iteration, so existing rows are threaded on the first one after start
  (`import-legacy-drafts-threaded`). Draft-only threads have `lmt` 0, as webmail drafts
  synced from the server always had.
- Drafts made by `draft.create` (the `tern draft` CLI, agents): one the engine saves with
  recipients, subject, body or files in `fields` is not pristine unless `fields` says so,
  so it is pushed; with `save: false` (the desktop) the caller decides. Found on a real
  Gmail account, 2026-10-21: a reply created with a body stayed pristine, was never pushed,
  and the desktop discarded it when its thread was opened (`draft-create-pushed`).
- Scenarios: `draft-push-{basic,replace,coalesce,pristine,no-uidplus,courier,gmail,reply-from-server}`, `draft-create-pushed`,
  `draft-send-after-push`, `draft-send-before-scan`, `draft-destroy-after-push`,
  `draft-destroy-refused` (risk 5), `draft-attachments` (with the `dovecot-appendlimit`
  personality), `draft-remote-edit-adopt`, `draft-conflict-local-wins` (slow),
  `draft-remote-delete`, `import-legacy-drafts-pushed`. `ab_known.py` classifies them as
  deliberate. `server_has` gained `contains` / `excludes` (full message text), and a
  `same_thread` expectation.
- Not done: risk 1 is open. No Gmail transcript was recorded; `draft-push-gmail` runs on the
  fake's Gmail model (APPEND to `[Gmail]/Drafts`, the copy in All Mail with `\Draft`), built
  from Google's IMAP extension docs. Run `test/tools/record_personality.py` against a real
  Gmail account before relying on it. CATENATE (risk 3) is not used.
