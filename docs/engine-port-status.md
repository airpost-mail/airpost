# Engine port status

State of the Rust port of Mailspring-Sync after P2 (plan section 8), 2026-10-15. P1 finished
2026-10-14. The plan is `docs/engine-port-plan.md`; this file records what exists and what the
next phases need.

## Workspace gate

All run from the repo root on the integrated tree:

| Check | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo test --workspace` | P2 tree: 397 pass, 0 fail; 41 `ms-imap` session tests and 1 `mailsync` cli test are `#[ignore]` (need Docker / 30 s) |
| `cargo build --release` | ok |
| `cargo deny check licenses bans` | ok; one reqwest (0.13, rustls/ring), no aws-lc-rs, OpenSSL or native-tls |
| P0 gate (migrate + sync through the harness's `tern-mailspring-bin/mailsync` symlink) | migrate prints `{"error":null}`; sync starts `main`, `background`, `foreground`, stays up, stderr empty |
| `pytest apps/engine/test --collect-only` | 215 tests collected |
| Upstream C++ binary, `--servers fake -k "baseline-initial-sync or metadata-expiration"` | 4 passed, 5 skipped (dovecot/cyrus not selected) (P1) |
| Full harness suite, Rust release binary, all servers (P2) | 121 passed, 90 failed, 3 skipped, 1 xfailed; no P2 regressions (see "Full-suite tally") |

The background and foreground workers are implemented (P2). Tasks are not: `queue-task` is
accepted and dropped, so every scenario that waits on a task times out.

## Packages

### P1-S store (`crates/ms-store`): gate passed

- What exists: V12 schema and `migrate`/`reset`; `Store` (save/remove/find/find_all/count, generic
  lookups, key-value, folder/label caches, `transaction` with delta batching, detached metadata);
  every model and its hooks; thread refcounting, ThreadCategory/ThreadCounts/ThreadSearch;
  placement helpers and `refreshMessageFromPlacements`; `DeltaStream` coalescing and flush timing;
  `invariants::check` porting all 11 checks of `invariants.py`.
- `Store::open` sets the SQLite temp directory on every open (main.cpp:919-922).
- `Contact::mutate_vcf_in_info(FnOnce(&str) -> Option<String>)` is the store half of
  `mutateCardInInfo`; P5 composes it with `ms_dav::VCard::{parse, incomplete, serialize}`.
- Tests: unit 4, delta 21, models 17, placements 27, schema 3, temp_dir 1.
- Open: `ContactGroup::afterRemove` leaves `gis` on members and `Contact::afterRemove` leaves join
  rows, as in C++. The `contacts` invariant (Python and Rust) now accepts those leftovers.

### P1-I IMAP and providers (`crates/ms-imap`, `crates/ms-providers`): gate passed

- What exists: `ImapSession` trait and `Session`, an own parser and command layer on tokio +
  rustls (the spike ruled out async-imap/imap-proto, `docs/spikes/imap.md`). TLS/STARTTLS,
  LOGIN/XOAUTH2, COMPRESS, QRESYNC/CONDSTORE, NAMESPACE, ID, VANISHED collection, MESSAGELIMIT,
  lenient ENVELOPE with the FullHeaders retry, APPEND with CRLF normalisation, IDLE with a
  cross-thread interrupt, `C`-prefixed tags, verbose `sent`/`recv` logging with AUTH redacted.
- `ms-providers`: `ServerProfile` and `resolve`, one file per provider (17), folder-role tables,
  OAuth endpoints.
- Tests: ms-imap lib 19, scripted 14; session 41 (ignored by default; pass against fake, Dovecot,
  Cyrus and the personalities with `--ignored`); ms-providers 8; conformance 9.
- Open: a FLAGS-only unsolicited FETCH for a requested UID counts as a FETCH without ENVELOPE; it
  triggers the FullHeaders retry only if that UID also produced no message.

### P1-M MIME (`crates/ms-mime`): gate passed

- What exists: `parse_message` (rendered body, files, headers, partID/uniqueID numbering),
  `flatten_html`, `snippet`, `search_text`, header decoding, charset detection, the outgoing
  `build` / `build_filtering_bcc` (alternative/related/mixed, priority headers, Bcc).
- Snippet and search text keep cited blockquotes with `> ` prefixes, as C++ does
  (`flattenHTML()` = `flattenHTMLAndShowBlockquote(true)`); plan section 6 is corrected.
- Tests: unit 46, builder 11, corpus 1 (195 messages), mailcore_parser 1, mailcore_summary 1,
  rules 21.
- Open: the flattener ignores elements nested deeper than 512 levels (hostile input only; C++
  would use gigabytes). A plain part with N levels of `>` still produces N nested blockquotes in
  the stored HTML, as in C++. Outgoing `message/*` and `multipart/*` attachments are sent as
  base64 `application/octet-stream`; macOS C++ sent `.eml` as inline `message/rfc822`.

### P1-N net (`crates/ms-net`): gate passed

- What exists: HTTP helpers (200-209 window, `{"text": …}` fallback, curl error keys), OAuth
  refresh for Google/Microsoft with a 60 s margin and rotation callback, `MetadataBackend` with
  `LocalOnly`, the People API client.
- Endpoint overrides read the harness names: `MAILSYNC_GOOGLE_TOKEN_URL`,
  `MAILSYNC_MICROSOFT_TOKEN_URL`, `MAILSYNC_GOOGLE_PEOPLE_ROOT` (`Endpoints::from_env`).
- Tests: unit 7, http 9, oauth 12, people 8.
- `SyncWorker::configure` calls `token_for` before every worker iteration of an OAuth account;
  `google-people-initial-sync` gets through its `ProcessAccountSecretsUpdated` (rotated refresh
  token) step on the Rust binary. The People client is not called by any worker yet (P4-G).

### P1-T SMTP (`crates/ms-smtp`): gate passed

- What exists: `SmtpClient` on lettre plus its own connection layer, CRAM-MD5/DIGEST-MD5, per-write
  timeouts, error mapping to the `ErrorCodeToTypeMap` names, the self-test email with Message-ID.
  The Outlook XOAUTH2 variant is decided by the SMTP host (`ms_providers::smtp_uses_outlook_xoauth2`).
- Tests: unit 17, harness_sink 11 (python3 FakeSmtpServer), scripted_server 18.
- Open: `slow_upload_outlasting_the_timeout` assumes loopback socket buffers of 8 MB or less; a host
  with much larger buffers could make it flaky.

### P1-D DAV library (`crates/ms-dav`): gate passed

- What exists: `DavClient` (discovery, listings, sync-collection, multiget, writes), XML helpers,
  vCard (`VCard::parse/serialize/incomplete`, `ContactFields`), ICS field extraction on calcard with
  `rs`/`re`, SRV/TXT discovery with a known-provider table, `DavProfile`, 429/Retry-After.
- Per-request Bearer `TokenProvider`; `.well-known` probes go without credentials; a 412 on DELETE
  is re-checked with a getetag PROPFIND (404/410 means already gone).
- Overrides: `MAILSYNC_CALDAV_HOST` / `MAILSYNC_CARDDAV_HOST` replace the SRV lookup in
  `DnsResolver` (`env_host_override`); `MAILSYNC_GOOGLE_CALDAV_ROOT` replaces the Gmail host in
  `CalendarHome::gmail` (`gmail_caldav_host`).
- Tests: unit 37, golden 2 (ics 11, vcf 8 cases), servers 26.
- Open: callers must supply a caching token provider (DavClient calls it before every request).
  With a runtime entered, each request runs on a scoped thread; don't drive it from a task that
  shares its thread with other async work. Windows cross-check not run.

### P1-H harness (`apps/engine/test`): gate passed

- What exists: tag-agnostic regexes, `fresh-db-replaces-legacy` and `import-legacy-carries-metadata`
  in place of the `migration-*` scenarios, metadata-expiration, Google People (fake People API) and
  DAV (Radicale/Baikal in Docker) scenarios, new `wait: {log, count}` and `wait: {people_requests}`
  forms, the endpoint override table in `test/README.md`.
- The C++ binary still passes the unchanged scenarios it was run on.
- Open: DAV and People scenarios cannot be validated end to end until P4-G/P5 exist (the C++
  binary does not read the overrides). Fixed waits in `dav-create-update-delete` (20 s after the
  pass starts) and `google-people-sync-token-expired` (5 s settle) may need tuning; a log line for
  the end of a contacts pass would make the first exact.

### P2-B background and P2-F foreground (`crates/ms-sync`): gate passed apart from task-blocked scenarios and one environment failure

- What exists:
  - `worker.rs` (`SyncWorker`, crate-private): store + `Session` per thread, `configure` (OAuth
    token before each iteration), `sync_folder_uid_range` (heavy/light fetch, MESSAGELIMIT,
    the 1024 full-headers cap and `should_retry_truncated_scan`), `sync_folder_changes_via_condstore`
    (CONDSTORE and QRESYNC, VANISHED), `delete_vanished_uids`, body caching
    (`sync_message_bodies`, `sync_message_body`, `clean_message_cache`).
  - `background.rs`: `run_background` (`runBackgroundSyncWorker`), folder/label listing with
    roles, duplicate-entry merge, root and Snoozed folder creation through
    `Namespace::path_for_components`, folder deletion via `detach_messages_from_folder`,
    STATUS/CONDSTORE/QRESYNC/plain passes, UIDVALIDITY reset and relink, the per-folder
    orphan-sweep bound (`folderCoveredAt`), the `removeLateSentCopies` hook for Sent folders
    when `profile.late_sent_copies`, and starting the foreground thread once the folder list
    exists.
  - `foreground.rs`: `run_foreground` and `idle_cycle_iteration` (body requests, connect/login,
    remote tasks, VANISHED drain, CHANGEDSINCE or shallow scan of the inbox, bodies, IDLE or
    the no-IDLE wait), `ForegroundControl` (IDLE interrupt, `need-bodies` queue, condvar).
  - `mail_processor.rs`: `insert_fallback_to_update_message` (lookup first, constraint fallback
    to update under the lock), vanished/unassigned placement deletion, `detach_messages_from_folder(folder_id, pause)`,
    `refresh_messages{,_with,_in_open_transaction}`, `sweep_expired_orphans` (re-reads orphans
    under the lock), `retrieved_message_body`, mail contacts.
  - `mailbox_model.rs`: `FolderModel` and `GmailLabelModel` (`classify_folders`,
    `on_placement_upserted`, `sent_placement_folder`).
  - Worker error policy in `lib.rs`: non-retryable `SyncError` or a panic logs critical and
    aborts; retryable ones log, report a connection error when offline, log `--sleeping` and
    sleep `WORKER_SLEEP` (120 s) or until `wake-workers`.
- Changes outside ms-sync: `Namespace::path_for_components` modified-UTF-7 encodes components
  and completes a prefix lacking its delimiter (mailcore `IMAPNamespaceItem::pathForComponents`);
  `File::create` always writes `contentId` (null when absent, as the C++ non-const access did);
  `Store::delete_vanished_placements_in` and its chunked list variant dedupe UIDs, keeping order;
  the stdin loop reads raw bytes and drops non-UTF-8 or non-JSON lines (only EOF or an I/O error
  starts the 30 s wait and exit 141); ms-tasks has `cleanup_old_tasks_at_runtime` and the
  selection half of `remove_late_sent_copies`.
- Tests: ms-sync unit 7, mail_processor 5 (two-store insert race, syncedAt guard, displaced-UID
  orphan and sweep, sweep re-read under the lock, Gmail single placement; each ends in
  `invariants::check`); ms-tasks unit 2; ms-imap lib +1 (namespace paths).

#### P2 gate (Rust release binary, 2026-10-15)

| Scenario group | Servers | Result |
|---|---|---|
| `baseline-initial-sync` | fake:dovecot, fake:plain, dovecot:plain, dovecot:qresync, cyrus:plain, cyrus:fastmail, cyrus:default-ns | pass |
| `condstore-initial-sync-large`, `modseq-truncation`, `sparse-uid-space`, `two-folders-identical-messages`, `o365-duplicate-sent-copies` | fake:dovecot, dovecot:qresync, cyrus:fastmail | pass |
| `qresync-*` (bulk-expunge, two-expunges, wide-vanished-range, vanished-above-highest-uid) | fake:dovecot / fake:cyrus-vanished, dovecot:qresync, cyrus:fastmail | pass |
| `qresync-vanished-during-body-fetch` | fake:dovecot | pass |
| `uidvalidity-change` | fake:dovecot, fake:plain, dovecot:qresync, cyrus:fastmail | pass |
| `uidvalidity-change-large-mailbox` | fake:dovecot, fake:plain | pass |
| `orphan-sweep-waits-for-initial-walk`, `orphan-sweep-with-unreadable-folder` | fake:dovecot | pass |
| `remote-flag-changes`, `remote-move-destination-scanned-first` | fake:dovecot, fake:plain, dovecot:qresync, cyrus:fastmail | pass |
| `remote-move-to-archive` | fake:dovecot, fake:plain, dovecot:qresync, cyrus:fastmail, cyrus:default-ns | pass |
| `remote-move-while-task-in-flight` | fake:dovecot, fake:plain, dovecot:qresync, cyrus:fastmail | fail: ChangeStarredTask times out (P3-T); passes on C++ |
| `self-addressed-inbox-and-sent` | fake:dovecot, fake:plain, dovecot:qresync, cyrus:fastmail, cyrus:default-ns | pass |
| `plain-expunge-found-by-deep-scan` | fake:plain, dovecot:plain, cyrus:plain | pass |
| `proton-all-mail-duplicates` | fake:proton-bridge, dovecot:proton-like | pass |
| `gmail-labels` | fake:gmail | fail: trash ChangeFolderTask times out (P3-T); passes on C++ |
| `folder-roles-by-localized-name`, `gateway-duplicate-list-entries`, `list-refused-transiently`, `messagelimit-partial-fetch`, `netease-coremail-id-before-select`, `status-uidnext-zero-new-mail` | their personality | pass |
| `new-mail-during-idle` | fake:dovecot, fake:plain, dovecot:qresync, cyrus:fastmail | pass |
| `connection-dropped-during-command`, `connection-dropped-during-idle` | fake:dovecot, dovecot:qresync, cyrus:fastmail | pass |
| `mid-pass-foreground-tombstone` | fake:dovecot, fake:plain | pass |
| `heavy-fetch-truncation-backlog` | fake:plain, dovecot:plain pass; cyrus:plain fails | cyrus:plain fails the same way on C++ (see open issues) |
| `rules-ready-incoming`, `-across-restart`, `-self-addressed-send`, `-gmail`, `-inbox-copy-first` | their servers | fail: SendDraftTask / ChangeFolderTask time out, or wait for `Syncing sent message` (P3-T/P3-S); all pass on C++ |
| `rules-ready-pre-upgrade-messages` | fake:dovecot, dovecot:qresync, cyrus:fastmail | skipped: `ab/mailsync-a83cc9a` baseline is not built |

- Open:
  - `heavy-fetch-truncation-backlog@cyrus:plain` fails on both binaries in this environment:
    the harness's 2300-message APPEND to Cyrus took ~220 s, so the engine's pass after its
    120 s sleep found ~1230 new messages instead of 2300 and truncated once (the scenario
    expects 2 or 3 truncations). It passed in the previous round on the Rust binary. The
    scenario assumes the APPEND finishes inside one worker sleep.
  - `uidvalidity-change@dovecot:qresync` takes ~2 min: Dovecot sends BYE to both connections
    when UIDVALIDITY changes, and the background worker sleeps 120 s after the connection
    error, as C++ does. It passes.
  - The foreground's CONDSTORE path does not compare UIDVALIDITY; only the background pass
    resets it. Same as C++ (SyncWorker.cpp:509 is in `syncNow` only).
  - TLS failure advice (`tlsFailureAdvice`) on connect is not ported; it comes with `test`
    mode (`TODO(P2-T)` in foreground.rs).

## Full-suite tally (P2, 2026-10-15)

`MAILSYNC_BIN=target/release/mailsync python3 -m pytest apps/engine/test`, all servers (fake,
dovecot, cyrus, radicale, baikal), 16 workers, 31 min: **121 passed, 90 failed, 3 skipped,
1 xfailed** of 215. The 90 failures were rerun on the C++ binary: 77 pass there, 13 fail.

| Category | Count | Scenarios |
|---|---|---|
| P3-T / P3-S not done (task times out, task not in the DB, or waits for the send path) | 75 | `client-task-move` (5), `client-task-trash-and-expunge` (3), `folder-deleted-with-copies-elsewhere` (3, remote-delete half passes, DestroyCategoryTask times out), `gmail-labels`, `gmail-send-and-labels`, `mark-read-fans-out-to-all-placements` (4), `move-into-folder-holding-a-copy` (4), `move-rejected-by-server` (2), `move-with-permuted-copyuid` (4), `move-without-move-connection-drop`, `move-without-move-keeps-other-deleted` (3), `remote-move-while-task-in-flight` (4), `rules-ready-across-restart` (3), `rules-ready-gmail`, `rules-ready-inbox-copy-first` (2), `rules-ready-incoming` (4), `rules-ready-self-addressed-send` (4), `send-draft` (2), `sent-copy-cleanup-store-refused`, `sent-copy-multisend`, `sent-copy-multisend-late`, `sent-copy-saved-late`, `star-rejected-on-condstore`, `synced-draft-destroy` (3), `synced-draft-destroy-courier`, `trash-message-with-two-placements` (4), `trash-two-placements-without-uidplus`, `undo-before-remote-phase` (4), `undo-move-restores-placements` (6). All pass on C++. |
| P3-T + P4-M not done | 2 | `metadata-expiration-snooze`, `metadata-expiration-send-later` (their SyncbackMetadataTask times out). Pass on C++. |
| P4-G / P5 not done; fail on C++ too (it does not read the endpoint overrides) | 12 | `dav-initial-sync`, `dav-incremental-sync`, `dav-create-update-delete`, `dav-etag-conflict` (radicale, baikal each); `google-people-initial-sync`, `-incremental-sync`, `-sync-token-expired`, `-contact-tasks` (fake:gmail) |
| Fails on C++ too, environment | 1 | `heavy-fetch-truncation-backlog@cyrus:plain` (slow Cyrus APPEND, above) |
| P2 regression | 0 | |

Skipped: `rules-ready-pre-upgrade-messages` ×3 (baseline binary missing). Xfail:
`import-legacy-carries-metadata` (P6). The 30 non-scenario tests (conformance, service fakes,
PIM oracles) pass.

## Integration changes (P1)

- Plan: section 6 snippet note corrected; section 6 DAV notes add the 412 scope (contact PUT, event
  DELETE), CardDAV PUT to `<uid>.vcf`, People `metadata.deleted` tombstones, expired sync tokens and
  contact-group leftovers; section 7 lists the endpoint overrides; 4.2/4.4/4.5 record the IMAP spike
  outcome, the generic profile defaults and the libdav decision.
- `ms-net` endpoint env vars renamed to the harness's `MAILSYNC_*` names.
- `ms-dav`: `VCard::incomplete` (always false, as C++), CalDAV/CardDAV host and Google CalDAV root
  overrides.
- `ms-providers`: removed the unused `ServerProfile::outlook_smtp_xoauth2`.
- `plugin_metadata` invariant (Python and Rust) skips only null, `{}` and `[]`, matching
  `MailModel::afterSave`'s `size() == 0`; `contacts` invariant in Rust narrowed to match Python.
- `ms-core`: `calendar_sync_pass` doc names its scenarios; `tests/logmsg.rs` checks the anchored regex.
- Root `Cargo.toml`: dropped the unused `libdav` and `quick-xml`.

## What P2 needed to know (P1 handoff, kept for reference)

- **ImapSession** is ready. Build with `Session::new(ImapConfig::for_account(&account, auth))`; it
  owns a current-thread runtime and must not be called from inside another runtime, so fetch OAuth
  tokens (ms-net, blocking) before calling it and push refreshed ones with `set_auth`. Errors are
  `MailError`; convert with `into_sync("<call>")`. Selection is implicit per call. `take_vanished`
  drains VANISHED collected from every response. `idle` takes `ForegroundControl::idle_interrupt()`.
- **Profile flags** are read from `session.profile()` after login. The C++ behaviour comes from the
  defaults: `trust_copyuid_pairing=false`, `reselect_after_append=true`,
  `tolerate_idle_disconnect=true`, `noop_before_selected_status=true`,
  `vanished_upper_bound_uidnext=true`, `merge_duplicate_list_entries=true`,
  `late_sent_copies=true`. Don't add provider checks in workers.
- **Store** is ready: one `Store::open` per thread (it is `!Send`), `transaction("<fn>", …)` for
  batched deltas, placement helpers and `MessageAttributes`/`FetchableCopy` in `ms_store::placement`,
  `save_folder_status` for `localStatus`. The harness's `force_scans` writes `localStatus` into the
  live DB, so re-read Folder `localStatus` every pass. Run `ms_store::invariants::check` in worker
  tests.
- **MIME** is ready for `retrievedMessageBody`: `parse_message(raw)` gives body, plaintext flag,
  files, snippet and search text inputs.
- **Stubs to replace** (all `todo!("P2-…")`): `FolderModel` / `GmailLabelModel` (`classify_folders`,
  `on_placement_upserted`, `sent_placement_folder`), `MailProcessor` (`insert_fallback_to_update_message`,
  `delete_vanished_placements`, `delete_unassigned_placements`, `detach_messages_from_folder`,
  `refresh_messages`, `sweep_expired_orphans`, `retrieved_message_body`), and the bodies of
  `run_background` / `run_foreground`. The P0 "worker is not implemented" log lines go with them.
  All replaced in P2.
- **P3** reaches workers through `ms_sync::{RemoteTaskRunner, TaskRunnerFactory}`; `ms_tasks::TaskProcessor`
  is all `todo!("P3-…")`.
- **P5**: use `ms_dav::CalendarHome::gmail` (which honours `MAILSYNC_GOOGLE_CALDAV_ROOT`) rather than
  `ServerProfile::caldav_url`, and `DnsResolver` (which honours the host overrides) for discovery.

## What P3 needs to know

- **Where tasks enter.** `apps/engine/src/stdin_loop.rs` has three `TODO(P3-T)`s on the main
  thread's store: `cleanup_tasks_after_launch` at startup, `perform_local` in `queue-task` (the
  task JSON already has `v = 0`; the 300 ms debounced foreground interrupt after it is in place),
  and `cancel` in `cancel-task`. `ms_tasks::TaskProcessor::{cleanup_tasks_after_launch,
  perform_local, cancel}` are still `todo!`, so wiring them in before implementing them aborts
  the process on the first task (the panic hook logs critical and aborts).
- **Foreground loop.** `Foreground::idle_cycle_iteration` runs body requests, connect, login,
  then `run_remote_tasks`, then the inbox CHANGEDSINCE/shallow scan and IDLE. `run_remote_tasks`
  builds a runner with `ctx.tasks.remote_runner(&store, &ctx)`, calls
  `cleanup_old_tasks_at_runtime`, then reads `status = 'remote'` tasks in rowid rounds
  (`remote_tasks_after`) until a round is empty, so a task queued by a running task (SendDraft
  queueing SyncbackMetadata) runs in the same iteration. A `queue-task` interrupt ends IDLE, and
  the next iteration picks the task up.
- **RemoteTaskRunner contract** (`crates/ms-sync/src/tasks.rs`):
  - `perform_remote(&mut Task, &mut dyn ImapSession)` must save the task's final status and
    error itself and return `Ok` for task failures. An `Err` leaves the foreground iteration:
    a retryable error sleeps the worker 120 s, a non-retryable one aborts the process. C++
    `performRemote` caught everything per task; keep that.
  - `remove_late_sent_copies(sent, session)` is called by the **background** worker, with the
    background's store and session, once per pass for each Sent-role folder when
    `profile.late_sent_copies`. Its selection half is done; the removal (`_removeSentCopies` /
    `_removeMessagesResilient`) is `todo!("P3-S")` and is reachable only for messages the send
    path stamped with `_asc`, so it cannot fire until SendDraft exists.
  - The factory is `ms_tasks::Tasks` (passed in by the bin as `ctx.tasks`); a runner borrows the
    calling thread's `Store`, which is `!Send`.
- **Store from tasks.** Use the `&Store` the runner is given: the foreground's store in
  `perform_remote`, the main thread's in `perform_local`. Never open a second store on the same
  thread. For placement and snapshot changes, build `ms_sync::mail_processor::MailProcessor::new(&account, store)`
  and set `is_gmail` from `session.profile().mailbox_model == GmailLabels` and
  `.with_config_dir(&ctx.config_dir)` if it writes files (the `TaskProcessor` struct needs to
  carry the config dir; it does not today). Relevant entry points:
  `refresh_messages_with` (change rows and refresh snapshots in one transaction per 100),
  `refresh_messages_in_open_transaction` (inside your own `store.transaction`),
  `detach_messages_from_folder(folder_id, Duration::from_millis(300))` for ExpungeAllInFolder
  (the background caller passes `Duration::ZERO`), `delete_vanished_placements_in`, and
  `insert_fallback_to_update_message` for a message fetched after APPEND/MOVE. Keep the
  placement rules in `legacy/mailsync-cpp/CLAUDE.md` ("Message Identity and Placements"): a
  displaced UID orphans, an unplaced message is kept as an orphan unless the caller asks for
  `UnplacedMessages::Remove`, and the end-of-pass sweep honours `folderCoveredAt`.
- **IMAP from tasks.** The session passed to `perform_remote` is the foreground's logged-in
  `Session`; selection is implicit per call (`store_flags`, `store_labels`, `move_uids`,
  `copy_uids`, `expunge`, `expunge_uids`, `append`, `find_uids_of_recent_header_message_id`,
  `create/rename/delete_folder`, `fetch_message_bytes`). A task that selects another folder
  leaves the session there; the foreground's CHANGEDSINCE fetch and IDLE select the inbox
  again. VANISHED are collected per path, so inbox expunges reported before a task switched
  folders are still drained by `take_vanished`. Read behaviour switches from
  `session.profile()` (`trust_copyuid_pairing`, `reselect_after_append`, ...), not from the
  provider. Map errors with `into_sync("<call>")`. `SyncWorker` is crate-private; if a task
  needs one of its helpers (body fetch, a UID-range scan of the destination), expose it from
  ms-sync rather than duplicating it.
- **Log lines** the task scenarios wait on (`logmsg::syncing_sent_message`,
  `logmsg::removing_late_sent_copies`, ...) already exist in `ms-core::logmsg`; use them, and
  check the scenario regexes before adding new ones.
