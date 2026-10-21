# Fix backlog (post-P6)

User-reported issues, root-caused on 2026-10-15 by read-only investigation. Fixes go into the Rust
engine and `apps/desktop`; the C++ engine stays as the A/B reference, so engine fixes that
change behaviour are recorded as deliberate divergences.
Paths: C++ relative to upstream Mailspring-Sync, desktop relative to `apps/desktop/`.

| # | Issue | Layer | Size | When |
|---|---|---|---|---|
| F1 | Duplicate Sent messages | engine + client | ~1–2 days | pause after P6 |
| F2 | Gmail Important markers | engine | ~2–3 days | pause after P6 |
| F3 | No reading pane for drafts | client | ~1.5–2 days | pause after P6 |
| F4 | Drafts are local only | engine + client | ~2–3 weeks | engine done 2026-10-21, `save_drafts_to_server` |

## F1. Duplicate Sent messages

The user runs Mailspring 1.26.0, which already has the upstream Sent-copy rewrite (Mailspring-Sync
#144, #150). The Rust engine ports that behaviour. Remaining causes, all present in both engines:

1. **Bcc changes the id.** The message id hashes To+Cc+Bcc (`MailUtils.cpp:639-652`,
   `crates/tern-core/src/ids.rs:84-95`). Our APPENDed Sent copy keeps Bcc; providers file the
   Bcc-stripped wire copy. A late provider copy then gets a different id, and
   `removeLateSentCopies` groups by id (`crates/tern-tasks/src/lib.rs:440-512`), so it never matches.
   Same for Bcc-to-self (Inbox copy vs Sent copy).
   - Fix: drop Bcc from the id source (D4 allows it; update the golden file). Also match late
     copies by `headerMessageId` in the cleanup.
   - Scenario: a `sent-copy-*` variant with a Bcc recipient (the fake SMTP already files the
     stripped copy, `apps/engine/test/fakeimap/smtp.py:193-196`).
2. **Quit mid-send leaves the draft row.** `_performRemoteRan` is saved before SMTP; the draft
   row is removed 4–12 s later (`crates/tern-tasks/src/send.rs:173-177`). Quitting kills the engine
   (`src/engine/client/bridge.ts:563-567`); on relaunch the task completes early and the draft stays.
   - Fix: engine removes the draft row in the early-return branch; client delays quit while a
     SendDraftTask is in flight.
   - Scenario: kill the engine during the Sent poll, restart.
3. **Late copy outside the 30 min window** (`SENT_COPY_CLEANUP_WINDOW`), or no UIDPLUS (no `_asc`
   written). Fix: key cleanup on `_asc` presence with a multi-day expiry, small indexed table.
4. Hypothesis only: providers rewriting Message-ID/Date/recipients. Needs a pair of real copies.
5. Optional client safety net after the engine fixes: collapse same-`headerMessageId` rows in a
   thread (`src/data/stores/message.ts:280-312`).

Engine half done 2026-10-15 (items 1-3; scenarios `sent-copy-bcc`, `sent-copy-send-interrupted`,
`sent-copy-late-after-window`). Item 3 has no new table: the cleanup starts from the
account's date index over the 7-day window and filters on `_asc`, which avoids a schema
migration; revisit if it shows up in a profile.

Item 2 done 2026-10-21. A relaunched send whose task already ran removes the draft row and
its server copies when `_smtpDone` is set, or when Sent shows a copy with its Message-ID
(stopped during SMTP, or before `_smtpDone` was saved); otherwise the draft stays for the
user (`crates/tern-tasks/src/send.rs`, scenario `sent-copy-send-interrupted`, unit test
`a_send_stopped_during_smtp_drops_the_draft_once_sent_shows_the_message`). With "Save drafts
to server" on, a draft row that outlived its send anyway (one the C++ engine left, imported
with Import from Mailspring) is removed instead of pushed, and a copy already pushed is
deleted (F4, `draft-sent-leftover`, `import-legacy-sent-leftover`). The client half is not needed for the draft row: the desktop no longer kills the engine on
quit but sends `engine.shutdown` and holds `will-quit` until it returns
(`src/main/application.ts`), and the relaunch cleans up. `engine.shutdown` waits at most
`ACCOUNT_STOP_TIMEOUT_MS` (5 s) for an account's threads, so a send in its Sent poll can
still be cut; if the gap below matters, the fix is the engine waiting for an in-flight
SendDraftTask on shutdown, not the desktop delaying quit. Follow-ups:
- Item 5, the client safety net.
- A send stopped after SMTP but before the APPEND, on a provider that files no Sent copy
  (iCloud, Fastmail), still ends with no Sent copy; the relaunch only removes the draft,
  and the draft's metadata (open and link tracking) is not attached to whatever copy syncs.
- Item 4 is still a hypothesis.

To pin down on the user's side: which account, whether the duplicate is also in webmail Sent,
whether Bcc was used or the app was quit right after sending, and the `engine-<accountId>.log`
lines for one affected send.

## F2. Gmail Important markers

Works when `[Gmail]/Important` is listed with `\Important`. Breaks in three setups; the Rust port
copies all three (`crates/tern-providers/src/roles.rs`, `crates/tern-store/src/models/thread.rs:565-582`,
`crates/tern-tasks/src/messages.rs:169-177`, `crates/tern-sync/src/background.rs:833-916`):

1. Important hidden from IMAP in Gmail settings: no `important` role, icon hidden, every
   `\Important` X-GM-LABEL dropped (Mailspring #443). Re-enabling needs Rebuild Cache because
   thread labels aren't recomputed when a label appears or gains a role.
2. A user label named "Important": the X-GM-LABELS lookup matches label paths before roles and
   returns the first hit (`MailUtils.cpp:447-461`), so threads get a user chip and removal sends
   the wrong label.
3. `[Gmail]/Important` listed without `\Important`: no path fallback in `COMMON_FOLDER_NAMES`;
   becomes a chip on every important message; removal fails with `ErrorStore` (forum topic 227).

Fixes (Rust only): match backslash system labels by role first, then only `[Gmail]/` /
`[Google Mail]/` paths; map `[Gmail]/Important` to `\Important` on STORE; add
`[gmail]/important`, `[google mail]/important`, `[gmail]/starred` fallbacks; recompute thread
labels from stored `remoteXGMLabels` when a label is created or gains a role; a `gmail-important`
scenario covering all three setups plus add/remove via ChangeLabelsTask. Optional client hint
when a Gmail account has no `important` role.

To check on the user's account: Gmail Settings > Labels, "Show in IMAP" for Important (and
Starred); a user label literally named "Important"; whether the app shows no star or a removable
chip.

## F3. Reading pane for drafts

The Drafts sheet only defines a `list` layout (`src/features/draft-list/index.ts`), unchanged
since 2015 when drafts opened only in popouts (Mailspring #1718, #1396). Proposal:

- Split mode: a `DraftPreview` column with the inline `Composer` role component (as reply drafts
  in `message-list/lib/message-item-container.tsx`), a selected-items stack for multi-select,
  double-click still pops out, Enter focuses the composer.
- List mode: single click / Enter pushes a `Draft` sheet with a back button.
- Changes: `draft-list` (main, new `draft-preview.tsx`, list columns, store re-resolving focus by
  `headerMessageId`), `src/data/stores/workspace.ts` (`'draft'` focus branch), a key-order
  fix in `src/components/multiselect/list.tsx`, two opt-out props on the composer
  (`src/components/decorators/inflates-draft-client-id.tsx`, `composer/lib/composer-view.tsx`: no
  delete-if-empty on unmount, no focus steal), styles and strings. ~450 LOC.
- Risks: draft id changes when From changes (track by `headerMessageId`); same draft open in
  preview and popout (Mailspring #2158). Check: Delete with only the keyboard cursor may be a no-op.

## F4. Server-synced drafts

Never built upstream (Mailspring #1726: effort and conflict handling). Editing a server draft
today only changes the local row, and a later webmail edit can orphan and sweep the local edits.

Design summary (full design: `docs/drafts-sync-design.md`):
- Identity: a draft copy with a Message-ID gets `id = draft_id(aid, Message-ID)`, so our APPENDed
  copy, other devices' revisions and webmail drafts collapse onto one row.
- Push from the foreground loop, not the task: quiet window (~20 s, ~120 s with large
  attachments), skip pristine drafts and drafts with a queued send/destroy; APPEND `\Draft \Seen`,
  resolve the new UID (APPENDUID, or the send path's recent-Message-ID search), then delete older
  copies. Never zero copies.
- Conflicts: a newer remote revision is adopted when nothing local is unpushed; otherwise local
  wins and the foreign copy goes to Trash (recoverable). `DraftTombstone` table so sent/destroyed
  drafts don't come back.
- Per-account "Save drafts to server" setting; gate off by default so A/B and existing gates
  compare like with like.
- ~13 harness scenarios. First step: record a real Gmail transcript to confirm how `\Draft` is
  reported in All Mail (docs show `\Drafts`).
- ~2.5–3.5k lines, 2–3 weeks. Lands after parity.

Engine done 2026-10-21 (`docs/drafts-sync-design.md` section 6). The setting is
`save_drafts_to_server`, default on. The harness and A/B runs turn it off. Open items:
- Risk 1: no real Gmail transcript yet; Gmail is modelled in the fake from Google's docs.
- Client: the composer should take an adopted revision only in fields the user has not
  edited (`draft-editing-session.ts`), and a Preferences toggle for the setting.
