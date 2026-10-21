# Server-synced drafts (design)

Status: proposal, 2026-10-15. Lands after engine parity, behind a gate that defaults
off. Backlog item F4 in `docs/fix-backlog.md`. C++ paths relative to `legacy/mailsync-cpp/`,
desktop paths relative to `apps/desktop/`.

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
