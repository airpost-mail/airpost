# Engine port status

State of the Rust port of Mailspring-Sync after P1 (plan section 8), 2026-10-14. The plan is
`docs/engine-port-plan.md`; this file records what exists and what the next phases need.

## Workspace gate

All run from the repo root on the integrated tree:

| Check | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo test --workspace` | all pass; 41 `ms-imap` session tests and 1 `mailsync` cli test are `#[ignore]` (need Docker / 30 s) |
| `cargo build --release` | ok |
| `cargo deny check licenses bans` | ok; one reqwest (0.13, rustls/ring), no aws-lc-rs, OpenSSL or native-tls |
| P0 gate (migrate + sync through the harness's `tern-mailspring-bin/mailsync` symlink) | migrate prints `{"error":null}`; sync starts `main`, `background`, `foreground`, stays up, stderr empty |
| `pytest apps/engine/test --collect-only` | 215 tests collected |
| Upstream C++ binary, `--servers fake -k "baseline-initial-sync or metadata-expiration"` | 4 passed, 5 skipped (dovecot/cyrus not selected) |

The background and foreground workers are still P0 stubs, so no mail scenario passes against
the Rust binary yet.

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
- Tests: ms-imap lib 18, scripted 14; session 41 (ignored by default; pass against fake, Dovecot,
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
- Open: no worker calls `HttpClient` or `token_for` yet, so these paths are not exercised end to end.

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

## Integration changes

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

## What P2 needs to know

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
- **P3** reaches workers through `ms_sync::{RemoteTaskRunner, TaskRunnerFactory}`; `ms_tasks::TaskProcessor`
  is all `todo!("P3-…")`.
- **P5**: use `ms_dav::CalendarHome::gmail` (which honours `MAILSYNC_GOOGLE_CALDAV_ROOT`) rather than
  `ServerProfile::caldav_url`, and `DnsResolver` (which honours the host overrides) for discovery.
