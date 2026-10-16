# Labels on every account

Draft, 2026-10-16. The user confirmed the recommended answers in 0.1 the same day. The user wants Gmail-style labels on every
account, not only Gmail. Written against `main` at 1e7274c23 (branch `docs/labels`). Engine
paths are relative to the repo root, desktop paths to `apps/desktop/app/`. Background:
`docs/architecture.md` (the backend holds metadata, never mail), `docs/backend-plan.md` 3.5
and 4.1 (metadata protocol, outbox), `docs/engine-protocol-plan.md` (read API) and
`docs/desktop-client-plan.md` (stages this work has to fit around).

Marks: **V** = read or grepped in this tree; **W** = checked against a public source (URL
in section 3); **U** = not verified, to check before the stage that relies on it. **Qn** is
item n of 0.1.

## 0. Decisions

| # | Decision |
|---|---|
| L1 | One label model in the engine. Every account has `Label` rows and takes `ChangeLabelsTask`; the engine picks where a label is stored, per account: a **label store**. Clients never write a store directly, so the desktop, CLI, tern-mcp and web share one write path. |
| L2 | Four stores. `gmail`: X-GM-LABELS, unchanged. `keywords`: IMAP keywords on each message copy, when the server allows them (L8). `tern`: Tern metadata on the message, synced between the user's devices by the Tern backend when one is configured, local otherwise. `outlook`: Outlook categories through Microsoft Graph, a later stage (L12). One store per account, chosen once and kept (L9). |
| L3 | Accounts become hybrid. Folders, placements and archive-by-move stay as they are on every non-Gmail account; labels are a second, orthogonal set. Gmail keeps its label model and behaviour exactly. |
| L4 | No folder-copy mapping. A label is never a mailbox that holds copies: that duplicates messages on the server and lets flags drift between copies. Proton Bridge's `Labels/` mailboxes, which are copies by Bridge's design, stay folders in v1 (Q4). |
| L5 | Labels attach to messages and roll up to threads as on Gmail today: a thread carries a label while any of its messages does, with the existing `_refs` / `_u` counters, `ThreadCategory` rows and `ThreadCounts` (`crates/ms-store/src/models/thread.rs:279-460`, V). The UI applies and removes labels per thread, which applies to every message of the thread; new messages in a labelled thread don't inherit the label, as on Gmail. |
| L6 | Label ids. Gmail ids stay `folder_id(aid, path)` (no migration). Keyword labels get a deterministic id from the keyword, so every device and every reinstall agrees without a backend; a rename gives a new id, as a Gmail rename does today. `tern` labels get a random id at creation and keep it across renames. |
| L7 | Names: `Label.path` holds the display path in modified UTF-7 with `/` between levels, for every store, so the desktop's `displayName` (`category.ts:77`, V) and the sidebar's nesting work unchanged. The store-specific key (keyword atom, Graph category name) lives in a separate field. Nesting is by `/` only, as Gmail does. |
| L8 | Keywords are used when the INBOX's `PERMANENTFLAGS` (or `FLAGS`, as Zoho sends it) contains `\*`, the server profile doesn't turn them off, and the first write reads back as stored. Otherwise the account uses `tern`: Yahoo, AOL, Proton Bridge and Exchange (Outlook/Office 365 until L12) by their PERMANENTFLAGS alone (3.2). |
| L9 | The store is chosen at the account's first folder sync after the upgrade, recorded in `_State` (`label-store-<aid>`), and never switched silently. A later change (keywords stop working, the user picks another store) is an explicit migration task that rewrites every label to the new store. |
| L10 | Keyword naming follows Thunderbird: `$label1`..`$label5` are its five default tags with its names and colours, and keywords Tern creates use its current key derivation (3.1), so a tag created with the same name in both clients gets the same key. Tern shows every Thunderbird tag; Thunderbird shows a Tern label only once its user creates a tag of that name (it ignores unknown keywords, bug 731143). Protocol and client flags (`$Forwarded`, `$MDNSent`, `$Junk`, `$MailFlagBit*`, `Junk`, `NonJunk` and the like) are never labels (2.4). |
| L11 | A label registry holds what no store carries: colours for every store, labels with no messages yet, and names for `tern` labels. It is account-scoped Tern metadata with a new object type `label`, one row per label, synced like any other metadata when a Tern server is configured and local otherwise. |
| L12 | Outlook categories via Graph come after desktop-client-plan stage 12, when OAuth moves into the engine (T14), and only if the user wants them (Q3). Exchange's IMAP doesn't take custom keywords (verified for on-prem, strongly indicated for Exchange Online, 3.3), so until then Microsoft accounts use `tern`. |
| L13 | `Account.usesLabels()` keeps its meaning, "the Gmail label model", because plugins call it. A new `Account.supportsLabels()` is true once the engine reports a store for the account (2.6); it is false before the first folder sync and against an engine without the feature (the C++ engine), which keeps today's UI. |
| L14 | Search: `in:<label>` works from the first stage through `ThreadSearch.categories`; `label:` lands in the engine grammar with desktop-client-plan stage 9 (T6), not in the desktop parser. |
| L15 | Colours (7.1). The user picks a label colour scheme, from which every label gets a colour automatically, and can set any colour on a single label. Both are desktop settings, per device, keyed by label name. Chips and dots are drawn from the label's colour mixed with theme variables, never hard-coded colours, so they stay legible in every theme (Darkside included). Desktop only, so it ships first, for today's Gmail labels (stage C1). |

### 0.1 Confirmed with the user

Recommended answer first; the user accepted every recommendation on 2026-10-16.

1. **Rename on a keyword account.** Rewrite the keyword on every message (the server stays
   the truth, other devices see the new name without a backend), except `$label1`..`$label5`,
   whose rename only changes the display name in the registry? *Yes.* Display-only renames
   (Thunderbird's own model: key fixed, name in local prefs) leave the old key on the server,
   so a second device without a Tern server keeps showing the old name. The cost: a
   Thunderbird user's tag for the old key loses its messages, and they recreate it under the
   new name to get them back.
2. **Choosing the store.** Automatic (L8), with an advanced per-account override in
   Preferences > Accounts ("Store labels on the server / in Tern") that runs the migration
   task? *Yes.* Some users won't want Tern-only keywords visible to other clients; most
   won't care.
3. **Outlook and Office 365.** `tern` store now; Outlook categories through Graph after
   stage 12, as an opt-in that needs re-consent for `Mail.ReadWrite` and
   `MailboxSettings.ReadWrite`? *Yes, and decide on Graph after the `tern` store has been
   used for a while.* Graph means new consent for every Microsoft account, a second token
   per account, a per-folder delta loop, matching Graph messages to IMAP messages by
   Message-ID, and renames as create-retag-delete (3.3): about 2k lines for one provider.
   Exchange Online's IMAP lacking keywords is strongly indicated but not verified against a
   current tenant (3.3).
4. **Proton Bridge.** Keep `Labels/<name>` as folders and use the `tern` store, or treat
   Bridge's label mailboxes as a fifth store (COPY adds a label, expunge from the label
   mailbox removes it)? *`tern` for v1.* Bridge's mapping is copies by design, which L4
   rules out elsewhere, and the folder layout is still NEEDS-RECORDING in the fake server.
5. **Important on non-Gmail.** Map the Important toggle to `$Important` (RFC 8457) on keyword
   accounts, and to a `tern` system label elsewhere, shown only with
   `core.workspace.showImportant`? *Yes.*
6. **Thunderbird's five defaults.** Show `$label1`..`$label5` only once a message carries one,
   rather than creating all five on every keyword account? *Only when seen.*
7. **Unified label views.** One sidebar entry per label name across accounts, like the
   unified Inbox, with "apply label" from a unified view creating the label in an account
   that lacks it? *Yes, in stage L6.*
8. **Message-level labels.** Keep the UI per thread, as Gmail, and allow `messageIds` in
   `ChangeLabelsTask` only for the CLI and tern-mcp? *Yes.* The desktop task refuses
   `messageIds` today (`change-labels-task.ts:103`, V).
9. **Backend object type.** Add `label` to the backend's metadata object types for the
   registry (`docs/backend-plan.md` 3.5 accepts only `message`, `thread`, `contact`)? *Yes.*
   It is a one-line allow-list change on each side and keeps one row per label, so two
   devices creating labels at the same time don't overwrite each other.
10. **Hybrid sidebar.** Two sections per non-Gmail account, Folders then Labels, instead of
    one mixed section? *Two sections.* One section titled by the inbox's class
    (`sidebar-section.ts:217-228`, V) can't name both.
11. **Label colours.** Decided by the user, 2026-10-16: keep it simple, a scheme the app
    colours labels from, plus full per-label customisation (any colour). 7.1.

## 1. Today

Engine (V):
- `MailboxModelKind { Folders, GmailLabels }` (`crates/ms-providers/src/lib.rs:73`) picks
  `FolderModel` or `GmailLabelModel` (`crates/ms-sync/src/mailbox_model.rs`). On Gmail every
  LIST entry except All Mail, Spam and Trash becomes a `Label` row; elsewhere there are no
  labels.
- Labels share the Folder shape and columns (`Label` table: id, accountId, path, role,
  `crates/ms-store/src/schema.rs:32`); ids are `hash("<aid>:<path>")`
  (`crates/ms-core/src/ids.rs:121`). No colour is stored anywhere; the desktop derives a hue
  from the name (`category.ts:150`).
- Per copy, `MessageFolder.remoteXGMLabels` holds the sorted X-GM-LABELS
  (`crates/ms-store/src/placement.rs:41`); `Message.labels` is the union over copies
  (`placement.rs:243-276`). `Thread::adjust_label_refs` resolves each name with
  `label_for_xgm_label_name` (system names by role, others by exact path) and counts it.
- The IMAP layer already parses keywords: `ImapMessage.keywords` collects every non-system
  flag atom from FETCH FLAGS (`crates/ms-imap/src/session.rs:1347`). Nothing reads them.
  `MessageAttributes::from_flags` keeps only `\Seen`, `\Flagged`, `\Draft` and Gmail labels,
  so a keyword change never registers as a change (`placement.rs:104-143`).
- `SELECT` parses `UIDVALIDITY`, `UIDNEXT`, `HIGHESTMODSEQ`, not `PERMANENTFLAGS` or the
  untagged `FLAGS` (`session.rs:727-756`).
- `store_flags` takes `MessageFlags` bits only; `store_labels` writes X-GM-LABELS
  (`session.rs:1556-1590`).
- **Bug that matters here:** on servers without MOVE, `move_messages_resilient` COPYs and
  then `UID STORE FLAGS.SILENT (...)` with only `\Seen`/`\Flagged`/`\Draft`
  (`crates/ms-tasks/src/imap_ops.rs:195-217`). The replacing STORE wipes every keyword,
  `\Answered` and `$Forwarded` the copy carried; COPY already preserves flags (RFC 3501
  6.4.7). It has to become add/remove of the three bits before keywords hold labels.
- `ChangeLabelsTask` carries `labelsToAdd` / `labelsToRemove` as Label JSON; the local phase
  rewrites the copies' label lists, the remote phase runs `+/-X-GM-LABELS` per folder
  (`crates/ms-tasks/src/messages.rs:174-244`). `SyncbackCategoryTask` creates a Label on
  Gmail and a Folder elsewhere, decided by `is_gmail(session)` (`folders.rs:113-131`); it has
  no kind field.
- Metadata (`crates/ms-sync/src/metadata_worker.rs`) attaches per plugin to `message`,
  `thread` or `contact`; message ids hash the account id, date, subject, participants and
  Message-ID (`ids.rs:60-110`), so they are the same on every device. The backend's last
  push wins per (object, plugin) row (`docs/backend-plan.md` 3.5).
- Read API: `labels.list` returns Label rows and counts; `threads.list` takes `labelId`
  (`apps/engine/src/read_api.rs:116,234`).

Desktop (V, from the readers' map):
- `Account.usesLabels()` is `provider === 'gmail'` (`account.ts:190`); there is no
  `usesFolders()`. Call sites: mail-rule action sets (`mail-rules-templates.ts:144`), label
  chips (`mail-label-set.tsx:39`), the Apply Label button and `core:change-labels`
  (`category-picker/lib/toolbar-category-picker.tsx:51,76`), the category mapper
  (`preferences-category-mapper.tsx:97`), the old `mcp-server` plugin (deleted in
  desktop-client-plan stage 2).
- Places that assume labels *or* folders: the sidebar's one user section titled by the
  inbox's class (`sidebar-section.ts:170-248`); `TaskFactory.tasksForArchiving` and
  `tasksForMarkingNotSpam` switching on the inbox's class (`task-factory.ts:56,77`); the move
  picker treating a Label target as "replace all labels" (`move-picker-popover.tsx:185-195`)
  and listing labels among move targets (`:126`); the label picker assuming
  `SyncbackCategoryTask` creates a Label (`label-picker-popover.tsx:120-138`) while the move
  picker assumes a Folder (`:151-160`); `CategoryMailboxPerspective`'s drag and remove logic
  (`mailbox-perspective.ts:455-589`: label into All Mail removes it, folder into label moves
  to All Mail, removing from a label view removes `[label, inbox]`); the archive rule
  removing the Inbox label (`mail-rules-processor.ts:145`); `getCategoryByRole` returning the
  first match (`category-store.ts:85`).
- Already type-agnostic: the categories observable (folders and labels concatenated,
  `mailspring-observables.ts:46-63`), `CategoryStore`'s standard/user/hidden lists,
  `Thread.categories` joined through `ThreadCategory`, `CategoryMailboxPerspective.threads()`.
- Keymaps: Gmail template binds `l` (`core:change-labels`), `v` (`core:change-folders`),
  `g l` (a TODO no-op, `focused-perspective-store.ts:78`); the Outlook template binds
  `core:change-category`, which has no handler.
- Search parses `in:` only; the local backend matches it against `ThreadSearch.categories`
  (role or path per category, `thread.rs:181`).

CLI and tern-mcp (V): `tern labels` and `tern threads --label` read; writes go through
`tern task queue` with raw JSON. tern-mcp's `set_labels` is described as Gmail only and
refuses accounts where `uses_labels` is false (`apps/mcp/src/tools/write.rs:124,863`;
`uses_labels` is `provider == "gmail"`, else "has any label", `tools/mod.rs:360`).

Fake IMAP (V): stores any flag atom (`fakeimap/store.py:233`) and answers `SEARCH KEYWORD`
(`server.py:829`), so keyword round trips already work. `PERMANENTFLAGS` is hard-coded with
`\*` for every personality (`server.py:632`, though `Mailbox.permanentflags` exists,
`store.py:88`); there is no keyword limit and no way to drop keywords. The real Dovecot runs
maildir by default (`harness/servers/dovecot.py:120`), and Cyrus runs with the fastmail
config, which gives a real per-mailbox keyword limit (3.2).

## 2. Model

### 2.1 Label JSON

Same table and columns as today. New JSON fields, all optional so Gmail rows stay as they are:

```jsonc
{
  "id": "…", "aid": "a1b2c3", "__cls": "Label", "v": 3,
  "path": "Clients/Acme",        // display path, modified UTF-7, '/' between levels (L7)
  "role": "",                    // "" for user labels; "important" for $Important (Q5)
  "store": "keywords",           // gmail | keywords | tern | outlook
  "key": "clients/acme",         // keywords: the atom; outlook: the category name; absent for gmail and tern
  "color": "#3b82f6",            // from the registry (L11); absent = the desktop's colour (7.1)
  "system": false                // true for $label1..5 and Important (rename is display-only)
}
```

`path` stays queryable and keeps its role in `ThreadSearch.categories`, so `in:` works.
`store`, `key`, `color` and `system` are not queryable; the stage 7 allowlist doesn't change.

### 2.2 Ids

- `gmail`: `hash("<aid>:<path>")`, unchanged.
- `keywords`: `hash("<aid>\u0000kw\u0000<lowercased keyword>")`. No folder path can produce that input
  (paths can't contain NUL), so it never collides with a folder id. Two devices, or a rebuilt
  database, derive the same id from the server alone.
- `tern`: `tl` + 32 random hex characters at creation (under the 40-character id column).
  Two devices that create "Work" offline end up with two labels named "Work"; the registry
  merge (2.5) folds the later-created into the earlier one.
- `outlook`: `hash("<aid>\u0000oc\u0000<category name>")`.

### 2.3 System labels

- Gmail: unchanged (`\Inbox`, `\Important`, `\Sent`, `\Draft`, `\Starred` by role).
- Keyword and `tern` accounts have no Inbox, Sent or Drafts labels: those stay folders.
  Starred stays `\Flagged`, unread stays `\Seen`.
- `$Important` (keywords) or a `tern` label with role `important` backs the Important toggle
  when `core.workspace.showImportant` is on (Q5).
- `$label1`..`$label5`: `system: true`, Thunderbird's names and colours (3.1). Rename changes
  the registry's display name only (Q1).

### 2.4 Which keywords are labels

A keyword is a label when it is `$label1`..`$label5`, `$Important`, or doesn't start with
`$`, and isn't on a short denylist of non-`$` flags other clients set (`Junk`, `NonJunk`,
`NotJunk`, `JunkRecorded`, `Redirected`, `Forwarded`, `Old`). Everything else starting with
`$` is a registered or client flag (3.2: `$Forwarded`, `$MDNSent`, `$Junk`, `$NotJunk`,
`$Phishing`, `$SubmitPending`, `$Submitted`, `$MailFlagBit0..2`, `$hasattachment`, `$muted`,
`$followed`, …) and is ignored. Matching is case-insensitive (Cyrus compares that way, JMAP
mandates it, Thunderbird lowercases its keys).

The filter runs when copies are read, so only label keywords reach `MessageFolder`. Changing
the rule later needs a rescan (`lastDeep = 0` on every folder), which a migration can do.

### 2.5 Registry (L11)

Object type `label`, object id = the label id, plugin id `tern-labels`, one row per label:

```jsonc
{"name": "Clients/Acme", "color": "#3b82f6", "createdAt": 1760600000, "deletedAt": null}
```

- `gmail` and `keywords`: holds the colour and, for a label no message carries yet, its
  existence (keyword labels vanish from the server with their last message). `name` is
  ignored except for `system` keyword labels.
- `tern`: holds the name, colour and tombstone; the label exists only here.
- Merge: same name (case-insensitive) on two `tern` rows → the one with the smaller
  `createdAt` (then id) wins; the engine re-points message metadata from the loser and writes
  the loser's tombstone. Runs when a registry row arrives.
- Without a Tern server the rows stay in `ModelPluginMetadata` on this device (`LocalOnly`).

Engine side, `apply_metadata` accepts `label` and stores it against the account
(`metadata_worker.rs:151`, today it ignores unknown object types). Backend side, `label`
joins the allow-list in 3.5.

### 2.6 What the desktop sees

`labels.list` gains `store` per account (`{labels, counts, store}`), and the engine emits it
on the account (`account.list`, then the stage 11 registry entry). Until stage 11 the main
window's `AccountStore` caches it per account from `labels.list` at start and from a
`Label` delta's `store`. `Account.supportsLabels()` reads it; `usesLabels()` stays
`provider === 'gmail'` (L13).

### 2.7 Per-store mapping

| | gmail | keywords | tern | outlook (later) |
|---|---|---|---|---|
| Lives on | each copy (All Mail) | each copy, per folder | the message id (metadata) | the Graph message |
| Read | X-GM-LABELS in FETCH | FLAGS in FETCH, filtered (2.4) | `tern-labels` message metadata `{"labels": [ids]}` | `categories` in Graph delta |
| Write | `±X-GM-LABELS` | `UID STORE ±FLAGS.SILENT (kw)` | metadata write + push | `PATCH /me/messages/{id}` |
| Other clients see it | yes | yes, if they show keywords | no | yes (Outlook, OWA) |
| Needs a Tern server for multi-device | no | no (colours yes) | yes | no (colours yes) |
| Rename | server rename | retag every copy (Q1) | registry only | `masterCategories` rename |
| Empty label | server keeps it | registry only | registry | server keeps it |

### 2.8 Engine shape

One `LabelStore` trait in `ms-tasks` (or a new `ms-labels` module beside it), one implementation per
store: `GmailLabels`, `KeywordLabels`, `TernLabels`, later `OutlookLabels`. The account's store
(L9) picks the implementation; `ChangeLabelsTask`, `SyncbackCategoryTask {kind: "label"}`,
`DestroyCategoryTask {kind: "label"}` and the sync passes call the trait and never branch on the
provider. Roughly: `labels_of(copy)` (read from a fetched message or metadata), `apply` /
`remove` (local and remote phase), `create`, `rename`, `delete`, `label_id(name)`. The choice is
per account, not per provider: two IMAP accounts at different hosts differ by what the server
allows (4), so a Fastmail account gets `KeywordLabels` and a Yahoo account `TernLabels`. The
desktop, CLI and tern-mcp only see Label rows and the tasks (L1).

## 3. Server facts

From a web research pass on 2026-10-16. Search engines rate-limited part of it, so most
items come from source code, RFCs, vendor docs and real server transcripts.

### 3.1 Thunderbird tags

- **Storage.** A tag is a key plus local prefs `mailnews.tags.<key>.tag` (name), `.color`,
  `.ordinal`. On a message the keys are the `keywords` property, written to the server as
  IMAP keywords equal to the key. W:
  `hg-edge.mozilla.org/comm-central/raw-file/tip/mailnews/base/src/nsMsgTagService.cpp`,
  `.../mailnews/imap/src/nsImapMailFolder.cpp`.
- **Defaults.** `$label1`..`$label5` (Thunderbird lowercases them to `$label1`…): Important
  `#FF0000`, Work `#FF9900`, Personal `#009900`, To Do `#3333FF`, Later `#993399`. W:
  `mail/locales/en-US/messenger/messenger.ftl`, `mailnews/mailnews.js`.
- **Key derivation, current** (`AddTag`, bug 650623 in TB 153 and bug 2073409 in TB 159).
  W: `nsMsgTagService.cpp` tip, `bugzilla.mozilla.org/show_bug.cgi?id=650623`,
  `…?id=2073409`.
  1. A tag with this exact name already exists: reuse its key.
  2. UTF-8 bytes of the name: ASCII alphanumerics kept; other bytes 0x21-0x7E kept except
     `= ( ) [ ] { } % * " \ < > ; &`; everything else (space, those symbols, every non-ASCII
     byte) becomes `=%02x`. Space is `=20`, `=` is `=3d`.
  3. Empty result: `tag`.
  4. Lowercase the whole key.
  5. Longer than 50 characters (Dovecot's default limit): `&x-moz-` + 32 lowercase hex
     characters, the first 16 bytes of SHA-256 of the UTF-8 name.
  6. Collision with another name: append `_1`, `_2`, ….
- **Key derivation, before TB 153.** `" ()/{%*<>\"` replaced by `_`, then IMAP modified
  UTF-7, then lowercased (which corrupts the base64 part, so non-ASCII legacy keys can't be
  decoded), collisions get `A`, `AA`, …, no length cap. W: `nsMsgTagService.cpp` at
  8d173ea78066. So keys on real servers mix `$labelN`, legacy keys, `=xx` keys and
  `&x-moz-…` hashes: Tern decodes `=xx` keys to a name and treats the rest as opaque.
- **STORE filter.** Thunderbird silently drops keywords with non-ASCII bytes, bytes ≤ 0x20,
  or `% * " \ ( ) [ ] { } < > ;`. W: `mailnews/imap/public/nsIImapService.idl`.
- **No sync of names or colours** between Thunderbird installs: they are prefs and
  Thunderbird Sync doesn't carry prefs. W: `mail/app/profile/all-thunderbird.js`,
  `mail/services/sync/modules/engines/`.
- **Unknown keywords are not shown.** Thunderbird doesn't turn server keywords it has no tag
  for into tags (bug 731143, still NEW). W. So a label Tern creates is invisible in
  Thunderbird until the Thunderbird user creates a tag with the same name, which, with the
  current algorithm, yields the same key and joins up. Tern, the other way, shows every
  Thunderbird tag at once.
- **Capability.** Thunderbird writes keywords only when it has seen `\*` in PERMANENTFLAGS
  or FLAGS (Zoho sends it in FLAGS). Its junk keywords are `Junk` and `NonJunk`, rewritten
  to `$Junk`/`$NotJunk` on Yahoo. W: `nsImapServerResponseParser.cpp`, `nsImapProtocol.cpp`.

Consequences for Tern: generate keys with the current algorithm (vectors from
`nsMsgTagService.cpp` in the unit tests), compare keywords case-insensitively and create them
lowercase, decode `=xx` keys for the default display name (lowercase, since the key is), and
take the real-case name from the registry when there is one.

### 3.2 Keyword syntax and server limits

- **Syntax.** `flag-keyword = atom`: 7-bit ASCII without `( ) {`, space, CTLs, `% *`, `" \`
  and `]` (RFC 3501 9, kept in RFC 9051). No length limit in IMAP. JMAP (RFC 8621 4.1.1)
  narrows it to 1-255 characters of 0x21-0x7E without `( ) { ] % * " \`, case-insensitive,
  returned lowercase; Thunderbird keys fit inside that. W.
- **PERMANENTFLAGS** (RFC 9051 7.1): `\*` means new keywords can be created; a keyword not
  listed is ignored or kept for the session only; a server at its per-mailbox limit MUST
  send PERMANENTFLAGS without `\*` (example: `NO [LIMIT] At most 32 flags in one mailbox
  supported`). W.
- **Registered keywords** (IANA "IMAP and JMAP Keywords"): `$MDNSent`, `$Forwarded`,
  `$SubmitPending`, `$Submitted`, `$Junk`, `$NotJunk`, `$Phishing`, `$Important` (RFC 8457),
  `$MailFlagBit0..2`, `$autosent`, `$canunsubscribe`, `$followed`, `$hasattachment`,
  `$hasnoattachment`, `$hasmemo`, `$memo`, `$imported`, `$istrusted`, `$maskedemail`,
  `$muted`, `$new`, `$notify`, `$unsubscribed`. Not registered but common: `$label1..5`,
  `Junk`, `NonJunk`, `NotJunk`, `JunkRecorded`, `Redirected` (iCloud). W:
  `iana.org/assignments/imap-jmap-keywords`. Apple's `$MailFlagBit0..2` are flag colours,
  valid only with `\Flagged` (RFC 9979). W.

| Server | `\*` | Limits | Source |
|---|---|---|---|
| Dovecot | yes | `mail_max_keyword_length` 50, checked at creation. Maildir maps 26 keywords to filename letters but keeps the rest in its index, so 26 isn't an IMAP-level cap. sdbox/mdbox: none documented (U). | W: doc.dovecot.org settings and maildir pages; MailKit dovecot fixtures |
| Cyrus | yes, while slots are free | 128 user flags per mailbox, 100 creatable by clients; `IMAP_USERFLAG_EXHAUSTED` when full; `cyr_expire` frees unused ones; case-insensitive | W: cyrus-imapd `imap/mailbox.h`, `mailbox.c`, `index.c` |
| Fastmail | Cyrus, so yes (U for Fastmail itself) | as Cyrus | Fastmail's own "labels" are mailboxes with a copy per label, not keywords (W: fastmail.help 1500000278282, 360058753554) |
| Gmail | yes, and keywords persist | — | W: MailKit gmail fixture, Geary test. Tern keeps X-GM-LABELS. |
| iCloud | yes | long-term persistence of custom keywords U | W: 2024 transcript (dzfrias/website) |
| Yandex | yes | — | W: MailKit yandex fixture |
| Zoho | `\*` in FLAGS, not PERMANENTFLAGS | U | W: imap-proto comment |
| Yahoo / AOL | no | — | W: senders.yahooinc.com developer documentation |
| Proton Bridge | no | labels are mailboxes | W: proton-bridge `internal/services/imapservice/connector.go` |
| Exchange on-prem 2007-SE | no | "Microsoft Exchange does not support client-defined keywords"; `$MDNSent` only | W: MS-STANXIMAP (May 2025); 2020 on-prem log |
| Exchange Online | no, by all evidence | — | 3.3 |
| GMX / Web.de | U | U | no transcript found |

Consequences: PERMANENTFLAGS alone sends Yahoo, AOL, Proton Bridge and Exchange to `tern`
without a profile rule. Tern also accepts `\*` in FLAGS (Zoho), as Thunderbird does, and the
read-back check (4.4) guards both. The realistic limit is Cyrus's 100 per mailbox; Tern keys
stay within 50 characters.

### 3.3 Exchange Online and Outlook categories

- **IMAP.** Exchange doesn't support client-defined keywords (MS-STANXIMAP, on-prem, W). An
  Office 365 transcript from 2017 shows `PERMANENTFLAGS (\Seen \Answered \Flagged \Deleted
  \Draft $MDNSent)` (W: bugzilla.mozilla.org 1407280 comment 4), and Microsoft Q&A (2024) and
  a 2017 Stack Overflow answer say categories aren't exposed over IMAP and O365 IMAP has no
  keywords (W: learn.microsoft.com/en-us/answers/questions/5332508,
  stackoverflow.com/questions/46914894). **Not verified against a current Exchange Online
  tenant**: MS-STANXIMAP doesn't cover EXO and I found no recent EXO transcript. The fake
  `outlook` personality should be re-recorded with `test/tools/record_personality.py` before
  `outlook-no-keywords` relies on it. imapsync's FAQ reports migrated mail "owning the
  category $MDNSent", so EXO may turn `$MDNSent` into a category on APPEND (U).
- **Graph.** `message.categories` is a list of category display names, set with `PATCH
  /me/messages/{id}` (`Mail.ReadWrite`). The master list is `/me/outlook/masterCategories`,
  `{displayName, color}`, colour `None` or `preset0`..`preset24`; `displayName` is unique and
  read-only after creation, so a rename is create, retag, delete. Listing needs
  `MailboxSettings.Read`, creating `MailboxSettings.ReadWrite`. Delegated for work and
  personal accounts, so Outlook.com is covered. Delta is per folder
  (`/me/mailFolders/{id}/messages/delta`); that a categories-only change shows up as
  `updated` is U. W: learn.microsoft.com Graph `outlookcategory`, `message`, `message-update`,
  `message-delta`, `outlookuser-list-mastercategories`.
- **Tokens.** The IMAP token's audience is outlook.office.com, so Graph refuses it. One
  authorize request can consent to IMAP and Graph scopes together, and the refresh token then
  redeems a separate access token per resource. Adding Graph mail scopes means a new consent
  for every existing Microsoft account, and needs Tern's own app registration
  (desktop-client-plan 0.1 item 18). W: learn.microsoft.com Entra access-token,
  auth-code-flow and refresh-token pages.
- **EWS** has `Categories` on items and the master list in a `CategoryList` user
  configuration (U); Graph doesn't reach on-prem Exchange, so on-prem categories would need
  EWS. Out of scope.
- **Prior art.** DavMail maps Thunderbird keywords to Exchange categories (`$label1..5` by
  name, others by config). W: sourceforge.net/p/davmail/feature-requests/72.

## 4. Capability detection

1. `SelectInfo` gains `permanent_flags` (parsed from `[PERMANENTFLAGS (...)]`) and
   `flags` (the untagged `FLAGS`). The background worker records `kw` (`\*` in either) in
   the folder's `localStatus` at every SELECT, so a mailbox that fills up (RFC 9051 7.1) is
   noticed at its next SELECT.
2. `ServerProfile` gains `keyword_labels: Auto | Off`. Off: Gmail (Gmail does take keywords,
   3.2, but its labels are X-GM-LABELS) and Microsoft hosts, as a guard should Exchange Online
   ever start listing `\*` without storing categories. Yahoo, AOL and Proton Bridge need no
   rule: they don't list `\*`.
3. At the first folder sync after the upgrade (or after account add), with no
   `label-store-<aid>` in `_State`: Gmail → `gmail`; INBOX `kw` true and the profile Auto →
   `keywords`; else `tern`. Recorded once (L9).
4. **Read-back check.** The first `ChangeLabelsTask` on a `keywords` account fetches FLAGS of
   one changed copy after the STORE. If the keyword isn't there, the task fails with
   `keywords-not-stored`, the engine records `label-store-check-<aid> = failed`, and the
   desktop offers to switch the account to `tern` (the migration task, 6). No silent switch.
5. **Per-folder.** A folder whose `kw` is false (read-only, shared, or full) can't take a label
   on its copies. `ChangeLabelsTask` skips those copies, keeps the label on the others, and
   reports `keywords-refused` with the folder paths in the task's error so the UI can say
   which folder. A STORE answered `NO [LIMIT]` (a full mailbox, as Cyrus does at 100
   client keywords, 3.2) is the same error.

## 5. Sync semantics

**Reading (keywords).** Keywords arrive with FLAGS in every fetch kind, so the existing
passes see them: CHANGEDSINCE on CONDSTORE servers (a keyword STORE bumps the modseq, RFC 7162
3.1.1), the shallow (2 min) and deep (10 min) attribute scans elsewhere
(`crates/ms-core/src/constants.rs:60-61`), and unsolicited FETCH during IDLE. The changes:
`MessageAttributes` carries the filtered keywords, `matches` compares them, and the placement
writes them into the per-copy label column. The column stays `remoteXGMLabels` in SQL until
the C++ engine is deleted (desktop-client-plan stage 10) and is renamed `remote_labels` in
Rust now; on non-Gmail accounts it is always `[]` today, so reusing it needs no migration.
The first scan after the upgrade updates every message that carries a label keyword, once.

**Writing (keywords).** `ChangeLabelsTask`'s local phase sets the label on every copy of
every message of the thread and holds the scan off with `_sa` as today; the remote phase runs
one `UID STORE +FLAGS.SILENT (kw)` / `-FLAGS.SILENT (kw)` per folder, in 50-UID chunks
(`STORE_FLAGS_CHUNK`). Add and remove are deltas, so a concurrent change by another client to a
different label survives; the same label changed on two clients ends as the last STORE.

**Copies.** A message with copies in two folders (self-sent mail in Inbox and Sent, iCloud's
duplicates) gets the label on both. `Message.labels` is the union, so another client removing
the keyword from one copy doesn't remove the label in Tern until the other copy loses it too.
Documented, not fixed: per-copy truth is what the server has.

**Moves and sends.** UID MOVE and COPY keep keywords (RFC 6851 3.3, RFC 3501 6.4.7); the
COPY-fallback STORE is fixed first (section 1). Sending a reply on a keyword account APPENDs
the Sent copy with the thread's label keywords, as `copy_thread_labels_to_sent` does on Gmail
(`crates/ms-tasks/src/send.rs:537`), so the thread stays labelled in other clients. Drafts get
no labels.

**Offline.** Tasks queue as today; local state shows the change at once; the remote phase runs
on reconnect. A message deleted on the server meanwhile drops out of the task, as for flags.

**Renames (keywords, Q1).** `SyncbackCategoryTask` with `kind: "label"` and `existingPath`:
local phase creates the new Label (new id, 2.2), moves the registry row's colour to it, and
rewrites every local copy's keyword; remote phase runs `+FLAGS (new)` then `-FLAGS (old)` per
folder over the copies the local database knows. A device that comes online later sees the
new keyword arrive and the old one go through its normal scans; the old Label row is removed
when no copy carries it and the registry has no row for it. Thunderbird sees a new tag key; its
old tag stays in its own prefs with no messages. `$label1..5` renames are registry-only.

**Deletes (keywords).** `DestroyCategoryTask` with `kind: "label"`: `-FLAGS (kw)` on every
known copy, then a tombstone in the registry. A client that re-adds the keyword brings the
label back; that is correct.

**`tern` store.** Labels are message metadata (`tern-labels`), written in the task's local
phase and pushed through the outbox (`docs/backend-plan.md` 4.1). The last push per message
wins at the server, so two devices changing different labels on the same message at the same
time lose one change; the value is small and the window is the push latency. A message whose
id has no date in its source (`ids.rs:76-84`) is keyed by folder and UID and loses its labels
when it moves; that is rare and is logged. Without a server, labels stay on the device.

**Thread rollup for `tern` labels.** A metadata change on a message saves the message, and the
save already diffs the snapshot into the thread (`thread.rs:279`); the snapshot's `labels`
gains the `tern` ids. `label_for_xgm_label_name` becomes a per-store resolver: Gmail as today,
keywords by `key`, `tern` by id.

**Several devices.** `gmail` and `keywords` need nothing from Tern: every device reads the
server. Colours and empty labels need the registry, so they sync only with a Tern server.
`tern` labels sync only with a Tern server.

## 6. Migration

- Gmail: none. Ids, rows, tasks and UI stay as they are; Label rows gain `store: "gmail"` on
  their next save, and readers treat a missing `store` as `gmail`.
- Non-Gmail accounts: none needed at upgrade. The first folder sync picks the store (4.3);
  keyword accounts pick up labels already on the server (Thunderbird tags show up at once).
- `LabelStoreMigrationTask {from, to}` (local phase marks the account busy; remote phase
  writes every label to the new store, then clears the old one): used by the Preferences
  override (Q2) and the failed read-back (4.4). Keywords → `tern` writes metadata from the
  local copies and removes the keywords; `tern` → keywords does the reverse. Idempotent, so a
  quit mid-way resumes.

## 7. Desktop UX

All gated on `account.supportsLabels()`. Gmail paths keep `usesLabels()` and don't change.

- **Sidebar.** Non-Gmail accounts get two sections, "Folders" and "Labels" (Q10), from
  `CategoryStore.userCategories(account)` split by class. Labels nest on `/` only (folders keep
  today's `[./\\]` sniffing). "New label" and "New sublabel" in the section's menu send
  `SyncbackCategoryTask {kind: "label"}`. A colour dot uses `label.color`, else the hue.
- **Toolbar, menu, keys.** "Apply Label" and `core:change-labels` (`l`) on every account that
  supports labels; "Move to Folder" (`v`) stays. The Outlook template's dead
  `core:change-category` binding is bound to the label picker. `g l` stays a no-op unless the
  user asks (not in scope).
- **Label picker.** Lists Labels without a role (as today) for the selected threads'
  accounts, plus Important when shown. "New" creates with `kind: "label"`, then applies. A
  colour swatch per row; "Edit colour" sends `SyncbackCategoryTask {kind: "label", color}`,
  so one task covers every label change.
- **Move picker.** On hybrid accounts it lists Folders only, so "move" never means "replace
  labels" outside Gmail (`move-picker-popover.tsx:126,185-195`).
- **Chips.** `MailLabelSet` shows Labels (not Folders) on hybrid accounts, with colour; remove
  queues `ChangeLabelsTask`. Gmail unchanged.
- **Perspectives.** A label view on a hybrid account lists threads through `ThreadCategory`
  as on Gmail. Archive (`e`) moves to the Archive folder and keeps the label;
  remove-from-view (`y`) removes the label; drag onto a sidebar label adds it; drag from a
  label view onto a folder moves and keeps the label. `tasksForRemovingItems` and
  `actionsForReceivingThreads` branch on `account.usesLabels()` instead of the category's
  class (`mailbox-perspective.ts:455-589`); `TaskFactory` the same (`task-factory.ts:56,77`).
- **Unified label views (Q7, stage L6).** A "Labels" group under the unified section, one
  entry per name (case-insensitive) present in two or more accounts, as a
  `CategoryMailboxPerspective` over every account's label of that name.
- **Mail rules.** Hybrid accounts get folder actions plus `applyLabel`; `moveToLabel` and the
  Gmail archive stay Gmail-only. Rules move to the engine in desktop-client-plan stage 14,
  which ports this action set.
- **Undo.** `ChangeLabelsTask` undo already swaps add and remove. `description()` says
  "Added Clients/Acme" / "Removed Clients/Acme" on hybrid accounts instead of the Gmail
  archive wording.
- **Search.** `in:Clients/Acme` works from stage L3; `label:` with stage 9 (L14). The search
  bar's suggestions already include labels (`search-bar-util.ts:53`).
- **Category mapper.** Unchanged: roles map to folders on hybrid accounts.
- **Plugins.** `Account.supportsLabels()` and `Label.color` are new exports; `usesLabels()`
  keeps its meaning. `docs/plugins-inventory.md` notes it.

### 7.1 Label colours (L15)

**Today (V).** `Category.hue()` sums the display name's char codes into a hue
(`src/flux/models/category.ts:156`); `LabelColorizer` (`src/components/mail-label.tsx:15-38`)
turns it into fixed `hsl()` lightnesses for the chip's text and background plus a white
gradient and inset shadow, and `backgroundColorDark` for the dots in the label and move pickers
(`label-picker-popover.tsx:86`, `move-picker-popover.tsx:109`) and the category mapper
(`category-selection.tsx:55`). Themes patch the result after the fact: ui-dark dims every chip
with `filter: contrast(110%) brightness(85%)` (`ui-dark/styles/ui-variables.less:67`), the
thread list greys chips on a selected row with `filter: brightness(600%) grayscale(100%)`
(`thread-list.less:30-36`), and Darkside paints them `@accent` on a focused row
(`darkside-labels.less`). The user can't choose a colour.

**What the user gets.** Two ways to colour labels, both in the desktop only:
- **A scheme.** Preferences > Appearance > "Label colours" picks one of a few schemes:
  Automatic (today's hue from the name, the default, so nothing changes unless the user picks
  another), Bright, Pastel, Muted, Earth, Grey. A scheme is a list of 12 colours; each label gets
  the colour at `hash(lowercased name) % 12`, so a label keeps its colour as labels come and go,
  and "Work" has the same colour in every account.
- **Per label.** "Colour..." in the sidebar label menu and on each row of the label picker: the
  current scheme's 12 swatches, a free colour picker (any colour), and "Reset" back to the scheme.
  A per-label colour beats the scheme.

**Storage.** Desktop config, per device: `core.appearance.labelColorScheme` and
`core.appearance.labelColors` (`{lowercased label name: "#rrggbb"}`). Keyed by name rather than
label id, so one choice covers the label in every account (matching the unified label views,
Q7) and survives a Gmail rename, whose id changes; the rename handler moves the entry to the
new name. No engine change, so it works for today's Gmail labels and later for keyword and
`tern` labels unchanged. Syncing colours between devices is left for later: it would go through
the Tern backend with the registry (L4), as a user-level setting.

**Legibility in every theme.** Every label resolves to one base colour (per-label colour, else
the scheme's, else `hsl(<hue()> 62% 47%)` for Automatic). The inline style sets only
`--label-base`; the stylesheet derives the chip and dot from theme variables:

```less
.mail-label {
  background: color-mix(in oklab, var(--label-base) var(--label-chip-tint), var(--label-surface));
  color: color-mix(in oklab, var(--label-base) var(--label-chip-ink-mix), var(--label-ink));
  box-shadow: inset 0 0 0 1px color-mix(in oklab, var(--label-base) var(--label-chip-edge), transparent);
}
.label-dot { background: color-mix(in oklab, var(--label-base) var(--label-dot-mix), var(--label-ink)); }
```

- Each theme sets `--label-surface` and `--label-ink` from its own LESS variables (background
  and text colour) and the mix percentages, in one `label-colors.less` per theme over a default
  in `static/style`. Scopes with a different background redefine surface and ink: Darkside's
  dark sidebar, a selected or focused thread row (surface = `@accent` or the selection colour),
  the dark message pane. That replaces the three filter rules above.
- Because chip text is mixed toward the theme's ink and the background toward its surface, a
  free colour stays readable: a dark blue picked in a light theme still gives light text on a
  dark-blue tint in ui-dark. The scheme colours are checked for contrast in every bundled theme;
  free colours aren't guaranteed, only kept close by the mix.
- `color-mix` and `oklab` are in Electron 44's Chromium (`apps/desktop/package.json:50`, V for
  the version). Mixing in oklab keeps perceived lightness even across hues, which `hsl()` doesn't.
- `LabelColorizer` keeps its exported functions for plugins (`color`, `backgroundColor`,
  `backgroundColorDark`, `styles`), returning the same `color-mix(...)` strings, and gains
  `baseColor(label)` and `dotStyle(label)`. The snooze chip (`snooze-mail-label.less`) uses the
  same variables. The gradient and white inset highlight go: they assume a light chip.

**Other clients.** Thunderbird's `$label1`..`$label5` start on Thunderbird's colours (`#FF0000`,
`#FF9900`, `#009900`, `#3333FF`, `#993399`, 3.1) unless the user sets one. Gmail's own label
colours live only in the Gmail API (W: Gmail API `users.labels`), which Tern doesn't use, so Tern
doesn't read or write them. Outlook categories (L7) carry one of 25 presets (W: Graph
`outlookCategory`): Tern shows the preset's colour unless the user sets one, and writes the
nearest preset when it does.

## 8. Engine, CLI and MCP surface

- `ChangeLabelsTask`: unchanged shape. The engine routes by store; `messageIds` accepted for
  the CLI and tern-mcp (Q8).
- `SyncbackCategoryTask` and `DestroyCategoryTask` gain `kind: "folder" | "label"` (default:
  `label` on Gmail, `folder` elsewhere, today's behaviour), and `color` on syncback.
- `labels.list`: `store` per account (2.6); Label JSON as 2.1.
- `LabelStoreMigrationTask` (6).
- CLI: `tern labels` prints colour and store; `tern label add|remove <label> <thread>…` and
  `tern label create|rename|delete` as thin task builders in `crates/tern-client`
  (desktop-client-plan T20) once that crate exists, else in `apps/cli`.
- tern-mcp: `set_labels` drops "(Gmail only)" and checks `store != none`; `uses_labels`
  becomes `labels.list`'s `store`; `list_labels` returns `store` and `color`.

## 9. Stages

Sizes are rough. "Stage n" without a prefix refers to `docs/desktop-client-plan.md` section 7.

| # | Stage | Depends on | Size |
|---|---|---|---|
| C1 | Label colours (7.1): schemes and per-label colours in desktop config, Preferences > Appearance picker, "Colour..." in the sidebar menu and label picker, per-theme `label-colors.less` (ui-light default, ui-dark, ui-darkside, ui-taiga, ui-less-is-more), `LabelColorizer` on `--label-base`, the filter rules removed, contrast spec. Today's Gmail labels | — | ~450 TS/LESS |
| L1 | IMAP groundwork: PERMANENTFLAGS and FLAGS in `SelectInfo`; `store_keywords`; keywords in `MessageAttributes`, `matches`, the per-copy column; COPY-fallback STORE fix; profile `keyword_labels`; fake IMAP per-personality PERMANENTFLAGS, keyword limit, drop mode | — | ~700 Rust, ~200 Python |
| L2 | Keyword store: store choice, label discovery from copies, Label JSON fields, per-store resolver in the thread rollup, `ChangeLabelsTask` / `SyncbackCategoryTask` / `DestroyCategoryTask` with `kind`, rename as retag, read-back check, sent-copy keywords, `labels.list.store`; tern-mcp and CLI changes | L1 | ~1.3k Rust |
| L3 | Desktop hybrid UI: `supportsLabels`, sidebar sections, picker, chips, perspectives, TaskFactory, rules action set, keymaps | L2; after stage 2 (mcp-server deleted) | ~900 TS |
| L4 | `tern` store and registry: `tern-labels` metadata, `label` object type in engine and backend, merge, `LabelStoreMigrationTask`, Preferences override | L2, L3 | ~1.1k Rust, ~50 backend, ~300 TS |
| L5 | `label:` in the engine search grammar | L2, stage 9 | ~150 Rust |
| L6 | Unified label views | L3 | ~300 TS |
| L7 | Outlook categories via Graph (Q3), colours as nearest preset (7.1) | L4, stage 12 | ~2k Rust |

Order: C1 first, now: it touches only the desktop's label rendering, theme stylesheets and two
config keys (`mail-label.tsx`, `extra.less`, `thread-list.less`, the pickers,
`category-selection.tsx`, the sidebar label menu, Preferences > Appearance, the `ui-*` themes),
none of which desktop-client stages 1, 2 or 6 change; the theme worktrees (`ui/theme`,
`ui/email-dark`) are checked with ternmail-ab before the `ui-*` files are edited. L1 and L2 any time; they touch the IMAP layer, placements and tasks, which no
desktop-client stage touches. L3 after stage 2, and before stage 14 so the rules port in 14
carries the hybrid action set; it touches category-picker, account-sidebar, mail-label-set,
TaskFactory and perspectives, none of which stages 1-13 change, so it can run beside them.
Stage 8 (DatabaseStore on `query`) doesn't care: labels are rows and joins it already serves,
and stage 7's parity fixtures should include a label view on a keyword account. L4's backend
part waits for the backend work to resume; `LocalOnly` works without it. L5 goes with stage 9
or right after it.

Each stage is one workflow with its gate: the quick harness tier while iterating, the full
suite and `cargo test --workspace` at the gate, and a trial check on `~/tern-trial` done by the
coordinator.

## 10. Tests

Fake IMAP (`apps/engine/test/fakeimap`), in L1:
- `Personality.permanentflags`, sent at SELECT from `Mailbox.permanentflags` instead of the
  hard-coded line (`server.py:632`): `outlook` as `(\Seen \Answered \Flagged \Deleted \Draft
  $MDNSent)`, `yahoo` and `proton-bridge` without `\*` (3.2), Zoho-style `\*` in FLAGS only.
  The `outlook` line is from 2017 and NEEDS-RECORDING against a current tenant.
- `keyword_limit` per mailbox: a STORE that would exceed it answers `NO [LIMIT]` and
  PERMANENTFLAGS drops `\*` once full (RFC 9051 7.1, Cyrus, 3.2).
- `drop_keywords`: STORE answers OK and keeps nothing, for the read-back check.
- Untagged `FLAGS` at SELECT lists the mailbox's keywords, as Dovecot does.

Harness scenarios (L2 unless noted), on `{fake: dovecot}`, `{fake: plain}` (no CONDSTORE),
`{dovecot: qresync}` (maildir, and once with `HARNESS_DOVECOT_FORMAT=mdbox`) and
`{cyrus: fastmail}` where they apply:
- `keyword-labels-basic`: apply, remove, undo; thread chips via `ThreadCategory`; counts.
- `keyword-labels-remote-change`: another client sets and clears keywords (Thunderbird keys and
  `$label3`); seen through CHANGEDSINCE, IDLE, and the shallow scan on `plain`.
- `keyword-labels-move`: move with MOVE and with the COPY fallback (`{fake: plain}` without
  MOVE) keeps keywords and `\Answered` (fails today).
- `keyword-labels-rename-delete`, `keyword-labels-two-copies` (self-sent mail).
- `keyword-labels-limit`: the 101st keyword in one mailbox on real Cyrus (`NO`, then
  PERMANENTFLAGS without `\*`), and the fake limit.
- `keyword-labels-dropped`: drop mode → `keywords-not-stored`, then the migration to `tern`
  (L4).
- `outlook-no-keywords`: the outlook personality picks `tern`.
- `gmail-labels`, `gmail-send-and-labels`: unchanged, in the gate.
- `invariants.py` and `ms_store::invariants` recount keyword label refs like Gmail's.
- Conformance probe `probe_keywords` on real Dovecot and Cyrus: STORE, FETCH, modseq bump,
  COPY and MOVE preservation, the untagged FLAGS list.

Unit: keyword filter and Thunderbird key derivation vectors (3.1, including spaces and
non-ASCII); resolver per store; label id derivation; the registry merge.

Backend (L4): `crates/ms-net/tests/backend_server.rs` gains a `label` row round trip and two
devices labelling the same message.

Colours (C1): a spec that computes the oklab mixes in TS for every scheme colour and a hue
every 15° (Automatic), against each bundled theme's surface and ink (selected rows and Darkside's
sidebar included), requiring 4.5:1 for chip text on the chip and 3:1 for dots on their surface
(WCAG 1.4.3, 1.4.11); scheme assignment stable by name; a per-label colour beating the scheme;
Automatic giving today's hue; a rename moving the per-label colour.

Desktop (L3): specs for the hybrid sidebar sections, picker on a keyword account, chips showing
labels but not folders, perspective remove and drag on a label view, `TaskFactory` archive on a
hybrid account, rules action set. Existing Gmail specs unchanged.

Trial (coordinator, `~/tern-trial`): a Dovecot or Fastmail test account (not the user's
production accounts) labelled from Tern and from Thunderbird side by side; a Gmail account to
check nothing changed.

## 11. Risks

- **Other clients wiping keywords.** Any client that STOREs `FLAGS (...)` without `+`/`-`
  replaces the whole set, as our own COPY fallback does today. Labels on a keyword account are
  only as safe as the least careful client the user runs. Documented; the read-back check
  doesn't catch later losses.
- **Server limits.** Cyrus (and so probably Fastmail) allows 100 client-created keywords per
  mailbox; a heavy label user can reach that in a large folder. The error names the folder;
  the user can switch the account to `tern` (6).
- **Keywords other clients don't show.** Thunderbird ignores keywords it has no tag for
  (3.1), Fastmail's web UI shows its own mailbox-based labels, not keywords (3.2), and Apple
  Mail and most mobile clients show none (U). A label applied in Tern is kept but invisible
  there. Users who expect to see Tern labels in Fastmail's web UI will be surprised; the
  Preferences text should say where labels live.
- **Hybrid confusion.** Archive keeps labels, remove-from-view removes one; on Gmail
  archive removes Inbox. The two models now sit side by side in a unified inbox. Keep the
  wording in undo toasts explicit.
- **Per-copy union.** A label removed by another client on one copy of a two-copy message stays
  in Tern (5).
- **First-scan delta burst.** The first scan after the upgrade re-saves every message carrying
  a label keyword; on a large Thunderbird-tagged mailbox that is many thread deltas at once.
  Batch through the normal scan transactions; measure on the trial.
- **`tern` store depends on stable message ids.** Ids hash the account id, so they match
  across devices only when the account id does (same email and server settings); a date-less
  message is keyed by location.
- **Last write wins per message in `tern`.** Concurrent edits on two devices lose one (5).
- **C++ engine.** None of this exists in C++; A/B scenarios that compare databases must skip
  keyword accounts until stage 10 deletes the C++ engine.
- **Plugin API.** A plugin that treats `!usesLabels()` as "no labels" won't show labels on
  hybrid accounts; it keeps working, it just doesn't gain the feature.

Open items:
- Fastmail's labels are mailboxes with a copy per label (3.2). Mapping them is the same
  folder-copy model L4 rules out; revisit only if Fastmail users ask.
- Keyword persistence on iCloud, Zoho, GMX/Web.de is U; the read-back check covers the
  first write, a trial account would cover the rest.
- `g l` (go to label) as a real command.
- Message-level label UI, if users ask (Q8).

## Status

2026-10-16: draft from a read of the engine (ms-imap, ms-sync, ms-store, ms-tasks), the
desktop (account, category models, tasks, picker, sidebar, chips, perspectives, rules,
search, keymaps), the CLI, tern-mcp, the fake IMAP server and the backend plan, plus a web
research pass on Thunderbird tags, keyword limits and Exchange (section 3). Nothing
implemented. Waiting for the user to confirm section 0.1.

2026-10-16: COPY-fallback fix (step 1) committed on `roadmap/labels` (5f203f1b1,
`move-without-move-keeps-keywords`). Section 7.1 (label colours, L15, stage C1) added at the
user's request: a scheme plus per-label colours, desktop only. C1 is next, ahead of L1.
The user confirmed questions 1-10 of 0.1 as recommended.
