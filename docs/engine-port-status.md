# Engine port status

State of the Rust port of Mailspring-Sync after P6 (plan section 8), 2026-10-15. P0-P6 are
implemented; what is left needs a human (real accounts, signing secrets, desktop checks,
decisions) and is listed under "Before shipping". P2 to P5 finished earlier the same day, P1
on 2026-10-14. The plan is `docs/engine-port-plan.md`; this file records what exists and what
is still open.

## Workspace gate

All run from the repo root on the integrated tree:

| Check | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo test --workspace` | P6 fix round 1: 481 pass, 0 fail, 42 ignored. P4/P5 tree: 469 pass, 0 fail, 42 ignored (41 `ms-imap` session tests and 1 `mailsync` cli test need Docker / 30 s). P3: 433 pass; P2: 397 |
| `cargo build --release` | ok |
| `cargo deny check licenses bans` | ok; one reqwest (0.13, rustls/ring), no aws-lc-rs, OpenSSL or native-tls |
| P0 gate (migrate + sync through the harness's `tern-mailspring-bin/mailsync` symlink) | migrate prints `{"error":null}`; sync starts `main`, `background`, `foreground`, stays up, stderr empty |
| `pytest apps/engine/test --collect-only` | 218 tests collected (P6; 215 at P4/P5) |
| Upstream C++ binary, `--servers fake -k "baseline-initial-sync or metadata-expiration"` | 4 passed, 5 skipped (dovecot/cyrus not selected) (P1) |
| Full harness suite, Rust release binary, all servers (P6 fix round 1, `--all`) | 215 passed, 3 skipped (`rules-ready-pre-upgrade-messages`, baseline binary not built) of 218, 9 min; see "Full-suite tally (P6)" |
| Harness quick tier (default `pytest apps/engine/test`, fake server, no Docker) | 106 of 218 cases, ~2.5-3 min |
| A/B, `ab.py batch`, Rust vs C++ 1.26.0, 188 cases | 0 real and 0 unexplained differences; `docs/engine-ab-report.md` |
| Full harness suite, Rust release binary, all servers (P4/P5 final) | 210 passed, 1 failed (conformance flake, passes on rerun), 3 skipped, 1 xfailed of 215; see "Full-suite tally (P4/P5)". P3: 196 passed, 15 failed |

The background and foreground workers (P2), the mail tasks, send and the `test` /
`install-check` modes (P3), the `calContacts` thread (Google People, CalDAV, CardDAV), the
contact, contact-group and event tasks, the metadata expiration worker and the remote metadata
worker (P4/P5) are implemented. P6 added `import-legacy`, the client engine setting, release
packaging, the harness tiers and the A/B report; see "P6". No phase has run against a real
account yet.

## Before shipping

Everything below needs a person: credentials, secrets, a running desktop app, or a decision.
Nothing here blocks further engine work; the fix backlog (`docs/fix-backlog.md`, F1-F4) can
proceed in parallel.

Desktop checks (not run here: no `node_modules`, npm was installed with `--ignore-scripts`)

- [ ] In `apps/desktop`: `npm ci`, then `npm run typecheck`, `npm run lint:check`, `npm test`.
  The P6 edits to `app/src/mailsync-process.ts`, `app/src/browser/application.ts`,
  `app/src/browser/app-window.ts`, `app/src/flux/mailsync-bridge.ts`,
  `app/src/config-schema.ts` and `app/spec/mailsync-process-spec.ts` were only syntax-checked.
- [ ] `apps/engine/packaging/build-release.sh macos --install-desktop`, then launch the app on
  a profile that has used C++. Check: the engine is `app.asar.unpacked/mailsync-rs`; the log shows
  `import-legacy` after `migrate`; `edgehill.legacy-imported.db` exists and
  `_State.legacy-import` is set; local drafts open with their attachments; a reply draft sits
  in its thread; an existing snooze and a send-later fire at their time; mail, contacts and
  calendars resync.
- [ ] Relaunch and confirm `import-legacy` logs nothing to import (it ran once).
- [ ] Profile move: the desktop now keeps its data in `Tern` / `Tern-dev` under the app-data
  dir and, on first launch, copies the `Mailspring` / `Mailspring-dev` profile there (no
  caches, crash dumps, locks or logs; the database via SQLite online backup; the Mailspring
  profile is only read). Check on a machine with Mailspring.app running: the first launch
  imports without a dialog, accounts sign in from the shared `Mailspring Safe Storage`
  keychain item, `migrate` and `import-legacy` run on the copy, Mailspring.app keeps
  working, and a second launch does not import again.

Real accounts (each: `--mode test` on both binaries, then a dogfood week with the Rust engine;
none has run against a real server)

- [ ] Gmail (OAuth): `--mode test` output against C++ (the open P3-A gate item), labels,
  send and Sent reconciliation, Google People contacts and contact tasks, Google CalDAV.
- [ ] Microsoft 365 / Outlook.com (OAuth, Outlook SMTP XOAUTH2): `--mode test` against C++,
  sync, send, late Sent copies (`removeLateSentCopies`).
- [ ] iCloud: IMAP, CardDAV, CalDAV (only Radicale and Baikal have run; iCloud has profile
  tests and fixtures only).
- [ ] Fastmail: IMAP (QRESYNC), CardDAV, CalDAV.
- [ ] Yahoo: IMAP, CardDAV (webmail root, empty-answer fallback), CalDAV.
- [ ] Optional: Nextcloud and GMX DAV, a generic IMAP host without IDLE or UIDPLUS.
- [ ] Body and snippet drift: sync a few thousand real messages with both engines and diff
  `Message.snippet`, `MessageBody` and `ThreadSearch` by `headerMessageId` (plan section 10).

Signing, CI and packaging

- [ ] Add the secrets and variables to the GitHub repo. Every signing, notarization, upload
  and publish step skips when its secrets are absent, so the workflows run without any.
  - Apple Developer ID (`engine-release.yml`, `desktop-build-macos.yml`):
    `APPLE_CODESIGN_P12`, `APPLE_CODESIGN_KEY_PASSWORD`, `APPLE_SIGNING_IDENTITY`. Without
    them the engine is signed ad hoc and the app is built unsigned; neither is notarized.
  - Notarization: `engine-release.yml` takes `APPLE_API_KEY_P8` / `APPLE_API_KEY_ID` /
    `APPLE_API_ISSUER` or `APPLE_ID` / `APPLE_ID_PASSWORD` / `APPLE_TEAM_ID`;
    `desktop-build-macos.yml` takes only the Apple ID set plus `APPLE_ID_ASC_PROVIDER`
    (electron-packager's `osxNotarize`), and `APPLE_PROVISIONING_PROFILE_B64`.
  - Windows Trusted Signing (`desktop-build-windows.yml`): `AZURE_TENANT_ID`,
    `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_TRUSTED_SIGNING_ENDPOINT`; variables
    `AZURE_TRUSTED_SIGNING_ACCOUNT_NAME`, `AZURE_TRUSTED_SIGNING_CERT_PROFILE_NAME`.
  - Uploads (all desktop builds): `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and the
    variable `BUILDS_S3_BUCKET` (the upstream workflows wrote to Mailspring's bucket);
    `SNAPCRAFT_STORE_CREDENTIALS` to publish the snap (its name is still `mailspring`);
    `SENTRY_AUTH_TOKEN`, `SENTRY_ORG`, `SENTRY_PROJECT` for source maps. Without them the
    builds are only workflow artifacts.
- [ ] Run `.github/workflows/engine-release.yml` once (`workflow_dispatch`, then an
  `engine-v*` tag). It has only been checked with actionlint and local runs of each step.
- [ ] Run each workflow once on GitHub. They are at the repo root now: `engine.yml` (fmt,
  clippy, cargo test, cargo deny, the harness quick tier, Windows `cargo check`),
  `backend.yml` (the same for `apps/backend`'s own workspace), `desktop-test.yml` (lint,
  typecheck, specs with the Rust engine installed; fails when no spec passes) and the
  manual `desktop-build-{macos,linux,linux-arm64,windows}.yml`. The macOS and Linux builds
  run `build-release.sh --install-desktop`, so they ship `mailsync-rs`. All were checked
  only with actionlint 1.7.12; none has run on Actions. The Linux install tests have not run
  against a package carrying `mailsync-rs`.
- [ ] Windows: there is no Rust build, so `desktop-build-windows.yml` packages no engine.
  It must not ship the C++ engine: its Windows build links the OpenSSL 1.1 DLLs in
  `legacy/mailsync-cpp/Vendor/libetpan/third-party/` (licence incompatible with the GPL), and
  the workflow fails if a `mailsync.exe` or OpenSSL DLL is in `apps/desktop/app`.
- [ ] Optional: build `apps/engine/test/ab/mailsync-a83cc9a` to un-skip
  `rules-ready-pre-upgrade-messages` (x3).

Licences and source (GPLv3 sections 4-6)

- [ ] Every published binary carries the licence and notices. Engine archives hold `mailsync`
  with `LICENSE.md`, `NOTICE.md` and `THIRD-PARTY-NOTICES.txt` (cargo-about, `about.toml`,
  `apps/engine/packaging/third-party-notices.sh`, which also appends the crates' NOTICE
  files). The desktop build (`build/generate-licenses.js`) writes
  `static/all_licenses.html` from the pruned app tree, the engine notices and the vendored
  assets in `build/resources/licenses/`, and puts `LICENSE.md`, `NOTICE.md` and
  `THIRD-PARTY-NOTICES-engine.txt` in `resources/`; on macOS also Electron's `LICENSE.electron`
  and `LICENSES.chromium.html`. In CI it fails when the engine ships without notices. The
  backend image has them in `/usr/share/doc/tern-backend`. Check one artifact of each kind
  after the first CI run, and open View License in a packaged app.
- [ ] Source offer: each desktop build tags its commit `desktop-<version>-<sha8>` and publishes
  `SOURCE.txt` (`.github/scripts/source-offer.sh`) next to the packages, in the workflow
  artifact and on S3. Engine releases (`engine-v*`) attach `SOURCE.txt` and
  `tern-engine-src-<tag>.tar.gz` (the Cargo workspace with every crate vendored). Keep the
  tags, the repository and the S3 objects available for as long as a build is offered. The
  desktop's npm dependencies are referenced by lockfile, not mirrored.
- [ ] npm packages that ship no licence file get their text from
  `build/resources/licenses/npm/` (`get-document`: MIT from upstream; `html-attributes`
  and `svg-attributes`: Open Works License 0.9.4, upstream repos gone) or the standard text for
  their declared licence. The build log names any package left without one; keep that list
  empty.
- [ ] The snap builds libsecret 0.20.5 (LGPL-2.1-or-later) from source and ships its COPYING;
  its source is the upstream tag in `snap/snapcraft.yaml`.

Decisions

- [ ] Rust-to-C++ rollback: local-only drafts and send-later/snooze values left in
  `edgehill.rust.db` are only warned about in the console, and the next switch overwrites that
  file. Pick one: a dialog before switching, timestamped copies, or an import on the C++ side.
- [ ] Connection-error reporting (P6 fix round 1): the Rust foreground no longer clears the
  error after an IDLE that ended in an error, so a drop of every socket at once (DarkWake) now
  shows as offline until a pass succeeds; C++ hid it (main.cpp:150). Keep or revert.
- [ ] Gmail DestroyContactGroupTask whose People API delete fails still loses the local group,
  as in C++ (P4-G open). Keep, or store the group and members in the task to restore it.
- [ ] Keep the P4/P5 departures from C++ recorded in plan section 6 "DAV" (If-None-Match on
  event creates, undo of refused SyncbackEvent / DestroyContact / CardDAV DestroyContactGroup).
- [ ] After the desktop rebrand, drop `mailspring` from the argv[0] check in
  `apps/engine/src/main.rs`.
- [ ] Schedule `docs/fix-backlog.md` F1-F3 (pause after P6) and F4 (after parity, D5).

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
- `SyncWorker::configure` calls `token_for` before every worker iteration of an OAuth account.
  The People client is used by `GoogleContactsWorker` and the Gmail contact tasks (P4-G), the
  `MetadataBackend` trait by `MetadataWorker` and SyncbackMetadata (P4-M).

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
- Open: the DAV and People scenarios have only ever run on the Rust engine (the C++ binary
  does not read the overrides), so they check the Rust engine against the C++ source, not
  against the C++ binary. The fixed waits in `dav-create-update-delete` (20 s after the pass
  starts) and `google-people-sync-token-expired` (5 s settle) are still there and passed in
  every P4/P5 run; a log line for the end of a contacts pass would make the first exact.

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
| `remote-move-while-task-in-flight` | fake:dovecot, fake:plain, dovecot:qresync, cyrus:fastmail | P2: fail, ChangeStarredTask times out (P3-T); passes since P3 |
| `self-addressed-inbox-and-sent` | fake:dovecot, fake:plain, dovecot:qresync, cyrus:fastmail, cyrus:default-ns | pass |
| `plain-expunge-found-by-deep-scan` | fake:plain, dovecot:plain, cyrus:plain | pass |
| `proton-all-mail-duplicates` | fake:proton-bridge, dovecot:proton-like | pass |
| `gmail-labels` | fake:gmail | P2: fail, trash ChangeFolderTask times out (P3-T); passes since P3 |
| `folder-roles-by-localized-name`, `gateway-duplicate-list-entries`, `list-refused-transiently`, `messagelimit-partial-fetch`, `netease-coremail-id-before-select`, `status-uidnext-zero-new-mail` | their personality | pass |
| `new-mail-during-idle` | fake:dovecot, fake:plain, dovecot:qresync, cyrus:fastmail | pass |
| `connection-dropped-during-command`, `connection-dropped-during-idle` | fake:dovecot, dovecot:qresync, cyrus:fastmail | pass |
| `mid-pass-foreground-tombstone` | fake:dovecot, fake:plain | pass |
| `heavy-fetch-truncation-backlog` | fake:plain, dovecot:plain pass; cyrus:plain fails | P2: cyrus:plain failed the same way on C++ (see open issues); passes since P3 fix round 1 (`hold_engine`) |
| `rules-ready-incoming`, `-across-restart`, `-self-addressed-send`, `-gmail`, `-inbox-copy-first` | their servers | P2: fail, SendDraftTask / ChangeFolderTask time out, or wait for `Syncing sent message` (P3-T/P3-S); pass since P3 |
| `rules-ready-pre-upgrade-messages` | fake:dovecot, dovecot:qresync, cyrus:fastmail | skipped: `ab/mailsync-a83cc9a` baseline is not built |

- Open:
  - (Fixed in P3 round 1 by `hold_engine`.) `heavy-fetch-truncation-backlog@cyrus:plain` failed on both binaries in this environment:
    the harness's 2300-message APPEND to Cyrus took ~220 s, so the engine's pass after its
    120 s sleep found ~1230 new messages instead of 2300 and truncated once (the scenario
    expects 2 or 3 truncations). It passed in the previous round on the Rust binary. The
    scenario assumes the APPEND finishes inside one worker sleep.
  - `uidvalidity-change@dovecot:qresync` takes ~2 min: Dovecot sends BYE to both connections
    when UIDVALIDITY changes, and the background worker sleeps 120 s after the connection
    error, as C++ does. It passes.
  - The foreground's CONDSTORE path does not compare UIDVALIDITY; only the background pass
    resets it. Same as C++ (SyncWorker.cpp:509 is in `syncNow` only).
  - (Fixed in P3.) TLS failure advice (`tlsFailureAdvice`) on connect: the foreground worker
    logs it and uses it as debuginfo (see P3-A).

### P3-T tasks and P3-S send (`crates/ms-tasks`): gate passed

- What exists: `TaskProcessor` (TaskProcessor.cpp) wired into the stdin loop
  (`cleanup_tasks_after_launch` at startup, `perform_local` on `queue-task`, `cancel` on
  `cancel-task`) and into the foreground worker through `ms_tasks::Tasks` (`perform_remote`).
  One module per task family, each naming its C++ source:
  - `messages.rs`: ChangeUnread, ChangeStarred, ChangeFolder (with undo through
    `undoPlacements` / `restorePlacements`), ChangeLabels; `imap_ops.rs`: `_resolveNewUIDs`,
    `_moveMessagesResilient`, `_removeMessagesResilient`, `_expungeMovedCopies`.
  - `drafts.rs`: SyncbackDraft (local only), DestroyDraft. `folders.rs`: SyncbackCategory,
    DestroyCategory, ChangeRoleMapping, ExpungeAllInFolder. `export.rs`: GetMessageRFC2822,
    GetManyRFC2822. SyncbackMetadata (local upsert; remote push only for a non-local backend),
    SendFeatureUsageEvent (no-op through `MetadataBackend::record_feature_usage`).
  - `send.rs`: SendDraft (single and multisend), Sent reconciliation, Gmail All Mail placement,
    `_removeSentCopies` (late-copy cleanup from the background worker); `rsvp.rs`: EventRSVPTask;
    `mailer.rs`: the SMTP factory (User-Agent "Tern").
  - `json.rs`: task field reads that fail with nlohmann's `what` text, so a malformed task ends
    as `unhandled-exception` on the task, as C++.
- Error policy: a task failure is saved on the task (`complete` + error JSON) and
  `perform_remote` returns `Ok`; only a SQLite error leaves the task as it was. Unknown
  classes complete, as C++. (The contact, contact-group and event tasks came in P4-G / P5.)
- Differences from C++: an unreadable draft attachment fails the send with `send-failed` (C++
  sent an empty part; plan section 6 "Send"). Attachments are read from the client's
  `files/xx/yy/<id>/<safeDisplayName>` path first (`File::client_path_in`), then the C++
  `safeFilename` path, then the only file in that directory.
- Tests: ms-tasks `tests/tasks.rs` 22 (moves with permuted COPYUID, MOVE-less moves, rejected
  move/star, undo before the remote phase, mark-read fan-out, draft destroy, category tasks,
  expunge-all, cancel and cleanup, send/multisend/SMTP failures, late copies, RSVP,
  attachments), unit 4; ms-store models +`safe_display_name_matches_the_client`.

### P3-A modes (`apps/engine/src/modes/`): gate passed for the harness accounts

- What exists: `--mode test` (`modes/test_auth.rs`, `runTestAuth`): IMAP connect, login and
  folder listing with the `inbox` (or Gmail `all`) role check, then SMTP login only when
  `settings.smtp_verification == "login"`, otherwise MAIL FROM / RCPT TO self and the test
  email; OAuth refresh first, with a rotated refresh token sent as `ProcessAccountSecretsUpdated`
  before the result. Output `{account, error, error_service, log}`, plus `error_advice` after a
  TLS failure and `error_offline` for failures outside the sessions; exit 0/1. `--mode
  install-check` (`modes/install_check.rs`). `ms_core::tls_failure_advice`
  (`MailUtils::tlsFailureAdvice`), also used by the foreground worker on a failed
  `connectIfNeeded` (`connectIfNeeded: <advice>` debuginfo, as SyncWorker.cpp:162-169).
- install-check targets: HTTP GET of the Google OAuth token endpoint (`MAILSYNC_GOOGLE_TOKEN_URL`
  overrides it; C++ pinged `$IDENTITY_SERVER/ping`), a TLS IMAP connect to imap.gmail.com:993
  with CAPABILITY, a TLS SMTP connect to smtp.gmail.com:465 with EHLO/QUIT. `tidy_check` and
  `legacy_tls_check` always succeed (no libtidy, no TLS below 1.2).
- Tests: `apps/engine/tests/test_mode.rs` (6: success, login-only SMTP, IMAP failures, SMTP
  failure, OAuth rotation, OAuth refusal); `apps/engine/tools/test_mode_ab.py` diffs both binaries.

| `test_mode_ab.py` (2026-10-15, P3 final run) | Result |
|---|---|
| fake:dovecot, fake:gmail, dovecot:qresync, cyrus:fastmail (12 cases each), dovecot:tls (6) | 52 match, 2 known divergences (both `dovecot:tls`), 0 mismatches |
| install-check | shape and outcomes match (both exit 0) |

The earlier P3-A run had 43 cases without fake:gmail (41 match, 2 known divergences).

- Known differences against the C++ binary:
  - `dovecot:tls` `good` and `self-signed-allow-insecure`: the shipped C++ build skips
    certificate validation when `imap_allow_insecure_ssl` is set and gives no advice for a
    certificate error; the Rust engine always validates and gives the certificate advice (as the
    C++ source does).
  - The advice never suggests "Allow insecure SSL" (`smtp_allow_insecure_ssl` /
    `imap_allow_insecure_ssl` are carried but have no effect); it says "Tern" where C++ said
    "Mailspring".
  - `log` transcripts differ in tag shapes, capability text and TLS notes; not compared.
- Not done: the manual Gmail/O365 `test` run against C++ (plan section 8 P3-A gate).

### P3 fix round 1 (2026-10-15)

- Attachments: `File::safe_display_name` / `client_path_in` port the client's
  `safeDisplayName` and `AttachmentStore.pathForFile`. SendDraftTask reads a draft file there,
  then at the C++ `safeFilename` path, then the only file in its directory; an unreadable file
  still fails the send with `send-failed` (plan section 6 "Send" records the change from C++,
  which sent an empty part). Inline parts downloaded with a body are written to the client path.
- Harness: `hold_engine: true` on a `server.*` step SIGSTOPs the engine while the step runs
  (`MailsyncProcess.held`); if the worker's 120 s sleep ran out meanwhile, it waits for the pass
  that starts on resume, because a `wake-workers` sent during that pass is not seen until the
  next sleep ends (both binaries). `heavy-fetch-truncation-backlog` holds the engine during its
  2300-message APPEND; its assertions are unchanged. It passes on cyrus:plain on the C++ and
  Rust binaries (append ~130 s).
- Full in-scope suite (fake, dovecot, cyrus; without `dav-*`, `google-people-*`,
  `metadata-expiration-*`, `import-legacy-carries-metadata`, `rules-ready-pre-upgrade-messages`),
  Rust release binary: **167 passed, 0 failed**; the 30 non-scenario tests pass.

### P3 fix round 2 (2026-10-15)

- No code changes. The manual Gmail/O365 `test` run against C++ (P3-A gate) is still not done:
  it needs real accounts, which this environment does not have.
- Full in-scope suite (same exclusions as round 1), Rust release binary: **197 passed, 0 failed**
  (167 scenario cases and 30 non-scenario tests, 11 min). `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` are clean; `cargo test --workspace`
  433 passed, 0 failed, 42 ignored.

### P3 final verification (2026-10-15)

- No code changes. Release binary rebuilt (up to date), full suite on all servers (below),
  `test_mode_ab.py --install-check` against C++ (53 of 55 match, 2 known divergences, 0
  mismatches; table above).
- Per-agent `target/p3-*` build dirs and old `apps/engine/test/runs/` session dirs removed;
  this run's artifacts are in `apps/engine/test/runs/p3-final/`.

### P4-G Google People (`crates/ms-sync/src/google_contacts.rs`, `crates/ms-tasks/src/contacts.rs`): gate passed

- What exists: `ms_sync::google_contacts::GoogleContactsWorker` (`GoogleContactsWorker.cpp`):
  `new(account, &Store, Arc<TokenManager>)`, `with_page_delay`, `run` (people and groups
  listings with `gsynctoken-contacts-<aid>` / `gsynctoken-groups-<aid>`, from
  `contacts_sync_token_key` / `groups_sync_token_key`), and the task writes `upsert_contact`,
  `delete_contact`, `upsert_contact_group`, `delete_contact_group`,
  `update_contact_group_membership`. `find_or_create_book` creates the `gpeople` ContactBook
  with the id the contacts pass uses, so a task queued before the first pass finds it.
- Tasks: `ms_tasks::CONTACT_TASKS` (DestroyContact, SyncbackContact,
  ChangeContactGroupMembership, SyncbackContactGroup, DestroyContactGroup) dispatch in
  `perform_local` / `perform_remote` to `contacts.rs`; Gmail goes to the People API, every
  other account to the CardDAV branch P5 added there (`DavWorker`). `TaskProcessor` carries
  `tokens` (from `ctx.tokens`; `new()` shares one TokenManager with the mailer) and `verbose`;
  `with_tokens` and `with_dav` (a `DavConnector` building the client and host resolver)
  replace them in tests.
- calContacts hooks: `cal_contacts.rs` holds `CalContactsWorkers { google, dav }`; a pass
  with contacts runs Google People, then `DavWorker::run` (`run_contacts`, the CardDAV half,
  skipped for Gmail; then `run_calendars`); a calendars-only pass and `sync-calendar`
  run `run_calendars`. Contacts every 75 min by elapsed time, calendars every 15 min, the
  failure policy of `runCalContactsSyncWorker` (`PassFailure`).
- Tests: ms-sync `google_contacts` 3, ms-tasks `contacts` 5, cal_contacts unit 2.
- Gate: `google-people-initial-sync`, `-incremental-sync`, `-sync-token-expired`,
  `-contact-tasks` pass on fake:gmail. No People scenario was changed.
- Open:
  - A Gmail DestroyContactGroupTask whose People API delete fails still loses the local
    ContactGroup, as in C++. Restoring it means saving the group and its members in the task
    data during the local phase (the join rows go on remove); needs a decision.
  - Only the fake People API has been exercised; no real Google account has synced.

### P4-M metadata (`crates/ms-sync/src/metadata_expiration.rs`, `metadata_worker.rs`): gate passed

- What exists: `MetadataExpirationWorker` (`new`, `with_chunk_pause`, `scan`) behind
  `run_metadata_expiration`, with the `METADATA_EXPIRATION_*` constants in ms-core;
  `ms_core::wake::DeadlineSignal` follows `isSavingMetadataWithExpiration` (`schedule`,
  `wake`, `wait_until`, `wake_time`, `is_waiting`). `MetadataWorker` (`new`, `cursor`,
  `pull_once`) and `apply_metadata` behind `run_metadata_worker`, which is not started for
  `LocalOnly`; `MetadataDelta::Cursor` lets a backend save a resume point mid-stream.
- Tests: ms-sync `metadata` 4, `apps/engine/tests/metadata_backend.rs` 3, wake unit 3.
- Gate: `metadata-expiration-snooze`, `-send-later` pass on fake:dovecot; the C++ binary
  passes them too and was the oracle. No metadata scenario was changed.
- Open: `MetadataWorker` is exercised only through test backends
  (`apps/engine/tests/metadata_backend.rs`); the shipped engine uses `LocalOnly`, so the worker
  never starts in production until a real backend exists.

### P5 DAV (`crates/ms-sync/src/dav.rs`, `crates/ms-tasks/src/events.rs`): gate passed

- What exists: `ms_sync::dav::DavWorker` (`DAVWorker.cpp`): CardDAV discovery with
  validation and rediscovery, sync-collection with the etag-listing fallback for address
  books and calendars, the Yahoo empty-answer fallback, 507 paging (in ms-dav), the
  empty-multiget token hold, the sync window for new events on the sync-token path, the
  deleted-master cascade, calendar metadata and pruning (only when the listing names a held
  calendar), `write_and_resync_contact`, `delete_contact`, `write_and_resync_event`,
  `restore_event_from_server`, `delete_event`; `rebuild_contact_group` and
  `ics_event_fields`. `ms_tasks::EVENT_TASKS` (SyncbackEvent, DestroyEvent) in `events.rs`.
- ms-dav changes: `DavProfile.carddav_webmail_root` (replaces the Yahoo bool),
  `double_encoded_hrefs`, `uppercase_group_kind`, `DavServer::Cyrus` / `Confluence`;
  UID-less VEVENTs get icalendarlib's placeholder UIDs (`is_placeholder_uid`); `VCard::is_group`
  compares the kind case-insensitively; `DavClient::create_ics`.
- Tests: ms-sync `dav` 12 (scripted transport: etag listings, sync-token edge cases, prune
  guard, event creates and restore), ms-tasks `dav_tasks` 5.
- Gate: `dav-initial-sync`, `dav-incremental-sync`, `dav-create-update-delete`,
  `dav-etag-conflict` pass on radicale and baikal.
- Scenario changes: one. `dav-incremental-sync` gained three `wait: {delta: …}` steps before
  its quiescence marks (each calendar's ctag persist, the changed calendar's ctag, the
  ContactGroup persist after the restart). Justification: every DAVWorker store streams its
  deltas with a 500 ms delay (`_streamMaxDelay(500)`, MailStore.cpp:95-96; only SyncWorker
  and main call `setStreamDelay`), so rows are in the database before their deltas reach
  stdout, and quiescence only sees deltas that have arrived. The assertions are unchanged.
- Open:
  - Items 18-20 of the review (If-None-Match on event creates; undoing a refused
    SyncbackEvent, DestroyContact or CardDAV DestroyContactGroup) depart from C++, which has
    the same gaps. Plan section 6 "DAV" records them; whether the plan keeps them needs
    confirming.
  - Only Radicale and Baikal have been run end to end. The other `DavProfile` servers
    (iCloud, Yahoo, Fastmail, Google, Nextcloud, GMX) have profile unit tests and recorded
    fixtures in ms-dav, not a real-server run.

### P4/P5 fix round 1 (2026-10-15)

- A new CardDAV group card gets no bookId until its read-back, as in C++
  (TaskProcessor.cpp:1528-1545): with one, an etag-listing pass between the local and remote
  phases deleted the card (empty etag, never listed) and its ContactGroup.
- A new event is PUT with `If-None-Match: *` (`DavClient::create_ics`); C++ sent no
  precondition.
- A refused remote phase no longer leaves the local edit behind (C++ did): SyncbackEvent
  re-reads the stored resource, or removes an event that was never written; DestroyContact
  unhides the contacts it did not delete (group cards stay hidden); a CardDAV
  DestroyContactGroup rebuilds the group from its card. A Gmail DestroyContactGroup that
  fails still loses the local group, as in C++. Plan section 6 "DAV" records these.
- DavWorker tests on ms-dav's scripted transport (`crates/ms-sync/tests/dav.rs`, helpers in
  `tests/support/dav_server.rs`, shared with ms-tasks); `Fake` gained `reset` and `on_body`.
- Full suite, Rust release binary, all servers (fake, dovecot, cyrus, radicale, baikal),
  without `import-legacy-carries-metadata` (P6) and `rules-ready-pre-upgrade-messages`
  (baseline not built): **211 passed, 0 failed** (12 min). `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace`
  469 passed, 0 failed, 42 ignored.

### P4/P5 final verification (2026-10-15)

- No code changes. Release binary up to date; full suite on all servers (tally below).
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` clean;
  `cargo test --workspace` 469 passed, 0 failed, 42 ignored.
- One leftover `harness-dovecot-*` container removed. Per-agent `target/p4-*`, `target/p5`,
  `target/p45-*` build dirs and the old `apps/engine/test/runs/` dirs (`p3-final`, `p4-*`,
  `p45-review-*`) removed; this run's artifacts are in `apps/engine/test/runs/p45-final/`.

## Full-suite tally (P4/P5, 2026-10-15)

`MAILSYNC_BIN=target/release/mailsync HARNESS_RUNS_DIR=apps/engine/test/runs/p45-final python3 -m pytest apps/engine/test`,
all servers (fake, dovecot, cyrus, radicale, baikal), 16 workers, 12 min: **210 passed, 1
failed, 3 skipped, 1 xfailed** of 215. Every P3 failure except the flake now passes.

| Category | Count | Cases |
|---|---|---|
| Harness flake, no engine involved | 1 | `conformance::test_fake_matches_dovecot[probe_idle]` (Dovecot's untagged `* OK` during IDLE missing under load, as in the P3 run); passed twice on rerun |
| P6 | 0 failed | `import-legacy-carries-metadata` is xfail |
| Fails on C++ too | 0 | |
| P4/P5 bug | 0 | |

Skipped: `rules-ready-pre-upgrade-messages` x3 (baseline binary `ab/mailsync-a83cc9a` not built).
Xfail: `import-legacy-carries-metadata` (P6). No case needed a C++ cross-check: the only
C++-oracle scenarios here (`metadata-expiration-*`) pass.

## Full-suite tally (P3, 2026-10-15)

`MAILSYNC_BIN=target/release/mailsync HARNESS_RUNS_DIR=apps/engine/test/runs/p3-final python3 -m pytest apps/engine/test`,
all servers (fake, dovecot, cyrus, radicale, baikal), 16 workers, 12 min: **196 passed, 15
failed, 3 skipped, 1 xfailed** of 215. Every P2 task/send failure now passes.

| Category | Count | Cases |
|---|---|---|
| P4-M not done (the expiration worker idles; the SyncbackMetadataTask before it completes) | 2 | `metadata-expiration-snooze`, `metadata-expiration-send-later` (fake:dovecot). Pass on C++ (rerun). |
| P4-G / P5 not done; fail on C++ too (it does not read the endpoint overrides) | 12 | `dav-initial-sync`, `dav-incremental-sync`, `dav-create-update-delete`, `dav-etag-conflict` (radicale, baikal each); `google-people-initial-sync`, `-incremental-sync`, `-sync-token-expired`, `-contact-tasks` (fake:gmail) |
| Harness flake, no engine involved | 1 | `conformance::test_fake_matches_dovecot[probe_idle]` (fake vs Dovecot untagged `* OK` text under load); passes on rerun |
| P3 bug | 0 | |

Skipped: `rules-ready-pre-upgrade-messages` x3 (baseline binary `ab/mailsync-a83cc9a` not built).
Xfail: `import-legacy-carries-metadata` (P6).

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

## P6

### import-legacy (`crates/ms-store/src/legacy_import.rs`, `apps/engine/src/modes/import_legacy.rs`)

- Implemented as plan section 9 describes: metadata re-keyed by Message-ID into pending
  `DetatchedPluginMetadata` rows, local drafts with body, File rows, files and metadata,
  `cursor-*`. `import-legacy-carries-metadata` is no longer xfail; `import-legacy-from-cpp-engine`
  and `import-legacy-twice` (both `binary: cpp`, slow) and `import-legacy-without-legacy-db`
  are new. Tests: `crates/ms-store/tests/legacy_import.rs` (11).
- After the commit the legacy database is renamed to `edgehill.legacy-imported.db*`, so a new
  `edgehill.db` created later without a new legacy move (the user deleted `edgehill.db*`)
  imports nothing.
- Reply and forward drafts join the thread of the message their `rthMsgId` / `fwdMsgId`
  names, at import if it is synced, otherwise through a `DraftThread` pending row when that
  message is first saved.

### Client engine (`apps/desktop/src/shared/engine-process.ts`)

- The desktop runs only the Rust engine, `mailsync-rs` in `app.asar.unpacked` (desktop-client
  stage 10 removed `core.sync.engine`, `MAILSYNC_ENGINE` and the C++ paths).
- `MailsyncProcess.migrate` runs `--mode import-legacy` after `migrate` on every launch; a
  failure is logged and the launch continues.

### Packaging

- `apps/engine/packaging/build-release.sh` (macOS arm64/x86_64/universal, Linux x86_64/arm64;
  `--install-desktop` copies the binary into the desktop app as `mailsync-rs`),
  `apps/engine/packaging/macos/{sign.sh,notarize.sh,entitlements.plist}` (hardened runtime,
  notarytool) and `.github/workflows/engine-release.yml` (tag `engine-v*` publishes a release;
  read-only token by default, third-party actions pinned to commits). No desktop workflow puts
  `mailsync-rs` into the app yet.
- Release builds accept an argv[0] containing `tern` or `mailspring`, because the app still
  ships as Mailspring.app.

### P6 fix round 1 (2026-10-15)

- `connection-dropped-during-command[fake:dovecot]` was flaky in two ways, both races between
  the background's `begin_connection_error` and the foreground's `end_connection_error` when
  one drop kills both connections: an end queued before the flush coalesced the `true` away
  (`[False]`), and an end interleaved inside a begin left the flag `false` with `true` queued,
  so the retry never cleared it (`[True]`). The foreground no longer clears the error after an
  iteration whose IDLE ended in an error (a departure from main.cpp:150, which hid the error
  from the client when every socket went stale at once), and begin/end set the flag and queue
  the delta under one lock.
- Desktop: `import-legacy` is run after `migrate` (above); `resolveSyncEngine` falls back to
  C++ when `mailsync-rs` is missing; the C++ switch-back warns about local-only rows.
- `import-legacy`: legacy database renamed after import; reply-draft threads (above).

### Harness tiers and timings

- `pytest apps/engine/test` runs the quick tier: 106 of 218 cases, fake server only, no
  Docker, ~2.5-3 min with 16 workers. `--slow` runs the other 112 (every Docker server kind,
  conformance probes, DAV self-tests, and scenarios marked `slow:`, today
  `import-legacy-from-cpp-engine` and `import-legacy-twice`); `--all` runs both, 9 min in fix
  round 1 (11 min 43 s for the whole suite before the tiers and the longest-first ordering).
  `--servers <kind>` runs that kind's cases in any tier. Rules: `apps/engine/test/README.md`
  "Tiers"; per-case durations in `apps/engine/test/durations.json`
  (`--record-durations`).

### A/B against the C++ binary

`python3 apps/engine/test/tools/ab.py batch` (2026-10-15, 17 min) ran 188 cases on the Rust
release binary and the C++ 1.26.0 binary and wrote `docs/engine-ab-report.md`: **0 real
behaviour differences, 0 unexplained**, 2863 deliberate (mostly `_State.engine = rust`, the DAV
and People cases the C++ binary can't reach because it ignores the endpoint overrides,
`fresh-db-replaces-legacy` and the Rust-only `import-legacy` mode) and 6 timing (thread search
body order when both workers fetch bodies). Classification rules are in
`apps/engine/test/tools/ab_known.py`. `--mode test` / `install-check` are compared separately
by `apps/engine/tools/test_mode_ab.py` (P3-A table).

### P6 final verification (2026-10-15)

- No code changes and no suite rerun (fix round 1 ran `--all`). One leftover
  `harness-cyrus-*` container removed; `target/p6-desktop`, `target/p6-legacy` and the old
  `apps/engine/test/runs/` dirs (`p45-final`, `p6-desktop`, `speed-before`) removed. Fix round
  1 kept no artifacts because every case passed (only failed and xfailed runs are kept).

## Full-suite tally (P6, 2026-10-15)

`MAILSYNC_BIN=target/release/mailsync python3 -m pytest apps/engine/test --all`, all servers
(fake, dovecot, cyrus, radicale, baikal), 16 workers, 9 min: **215 passed, 0 failed, 3
skipped, 0 xfailed** of 218. No case needed a rerun. `cargo test --workspace` 481 passed, 0
failed, 42 ignored; fmt and clippy (`-D warnings`) clean.

| Category | Count | Cases |
|---|---|---|
| Failed | 0 | |
| Skipped | 3 | `rules-ready-pre-upgrade-messages` (fake:dovecot, dovecot:qresync, cyrus:fastmail): baseline binary `ab/mailsync-a83cc9a` not built |
| Xfail | 0 | `import-legacy-carries-metadata` passes since P6 |

## What P6 needs (P4/P5 handoff, kept for reference)

- **`import-legacy`** (plan section 9) is the only missing mode; `import-legacy-carries-metadata`
  is xfail and becomes its gate. It reads `edgehill.legacy.db` (left by `migrate`) and imports
  `ModelPluginMetadata` for messages and threads as `DetatchedPluginMetadata`-style rows keyed
  by `headerMessageId`, local drafts (UID-0 drafts placements with `MessageBody` and files) and
  the `_State` keys that still apply. Snoozes and send-later must survive: the expiration
  worker (P4-M) only sees metadata that is attached to a row, so check an imported snooze
  wakes. Contacts, events and mail are not imported; the calContacts and mail workers resync
  them.
- **Packaging:** macOS signing and notarization of `target/release/mailsync`, and Linux
  packages. The client finds the binary at `<resources>/mailsync` under `app.asar.unpacked`
  (`apps/desktop/src/shared/engine-process.ts:135`). `cargo deny` is clean today; keep the
  reqwest/rustls (ring) stack free of OpenSSL and aws-lc-rs.
- **Client engine setting:** a setting that picks the C++ or Rust binary. `migrate` already
  moves a C++ `edgehill.db` aside when `_State.engine` is not `rust`. C++ opens a
  `user_version = 12` DB without migrating (MailStore.cpp:116, 212-257), so switching back runs
  C++ on the Rust-written DB; decide whether that is allowed or the switch forces a resync.
- **`ab.py` over the full suite:** `apps/engine/test/tools/ab.py run` on the C++ binary and the
  Rust one, then `compare`. Expect the DAV and People scenarios to differ (the C++ binary does
  not read the endpoint overrides) and the two `dovecot:tls` test-mode divergences; anything
  else is a regression. Building `ab/mailsync-a83cc9a` would also un-skip
  `rules-ready-pre-upgrade-messages`.
- **Dogfooding on real accounts**, which no phase has done yet: Gmail and O365 `--mode test`
  against C++ (the P3-A gate item), Gmail People and Google CalDAV, iCloud/Fastmail/Yahoo
  CardDAV and CalDAV, and a few thousand real messages diffed for snippet and body drift
  (plan section 10).
- **Decisions pending from P4/P5:** the Gmail DestroyContactGroup restore (P4-G open) and
  whether the plan keeps the P4/P5 departures from C++ in section 6 "DAV" (P5 open).

## What P3 needed to know (P2 handoff, kept for reference)

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
