# Porting mailsync to Rust

This is the brief for the port. It records the decisions already made, the contract with the
client, the architecture, and a work breakdown sized for a multi-agent workflow. A session
starting from scratch should be able to execute from this file plus the C++ source. `legacy/mailsync-cpp/CLAUDE.md` describes the C++ engine's design and invariants, especially
"Message Identity and Placements"; read it first. C++ paths in this doc (`MailSync/…`,
`Vendor/…`, `test/…`) are relative to `legacy/mailsync-cpp/`. Rust paths are relative to the
repo root (section 12).

## 0. Decisions

| # | Decision |
|---|---|
| D1 | Pure Rust. No mailcore2, libetpan, ICU, tidy, ctemplate, libxml2, curl, cyrus-sasl or OpenSSL; no C++ FFI. |
| D2 | Keep every Pro / Mailspring ID feature (snooze, send-later, reminders, open/link tracking sends, thread metadata). Remove the sign-in requirement: the engine runs with `--identity null` as a normal mode, not a degraded one. |
| D3 | Metadata sync goes through the `MetadataBackend` trait. `LocalOnly` (metadata stays in the local database) is the default; with a Tern server object on the identity line the engine uses `HttpMetadataBackend` against the self-hostable `apps/backend` for multi-device sync (`docs/backend-plan.md` 4.1). Nothing calls `*.getmailspring.com`. *(Changed 2026-10-16, backend B1-E.)* |
| D4 | Fresh database. The Rust engine never reads or migrates a C++-era DB in place. An optional `import-legacy` mode carries over local data that would otherwise be lost (section 9). Message ids no longer have to match the C++ engine byte-for-byte. |
| D5 | The client contract is frozen: CLI, stdin/stdout protocol and SQLite schema stay exactly as they are, so the existing harness can A/B the Rust binary against the C++ one. Protocol redesign comes after parity. |
| D6 | CalDAV/CardDAV and Google People contacts are in scope but come last. |
| D7 | macOS and Linux first. Windows comes after parity (section 11). |
| D8 | No feature cuts beyond ones that would pull C dependencies back in (section 2). |
| D9 | Provider-specific behaviour lives in provider profiles, one file per provider, not as scattered host checks (section 4). |
| D10 | One monorepo holds the engine, the desktop client, the future backend and shared libs. Name: Tern (GitHub org `ternmail`); see section 12. |
| D11 | Everything ships under GPL-3.0-only. Upstream is received under GPL-3.0-or-later (its in-app notice grants "any later version"), which permits distributing as GPL-3.0-only; NOTICE.md says so. The product is rebranded away from "Mailspring" (section 12). *(Wording changed 2026-10-16.)* |
| D12 | One Cargo workspace at the repo root covers the engine, the backend and every Rust library, with one `Cargo.lock`. The backend reuses no engine crate. It shares only `crates/ms-wire` (wire types) with the engine (`docs/backend-plan.md` 3.1). *(Changed 2026-10-16, backend B1-E.)* |

## 1. Size and expectations

The repo is about 640k lines of C/C++, but only about 20k of it is the engine (`MailSync/`).
The rest is vendored:

| Vendored code | Lines |
|---|---|
| libetpan | 159k |
| mailcore2 | 130k |
| SQLite amalgamation in SQLiteCpp | ~280k |
| nlohmann | 26k |
| spdlog | 11k |
| icalendarlib | 1.3k |
| StanfordCPPLib | 4.3k |

Crates replace all of the vendored code. The engine is dense invariant and quirk logic, so the
Rust version won't be dramatically smaller. Expect 15–22k lines of Rust that we own: 12–18k of
engine and 2–4k of IMAP glue.

Engine files by size, for planning:

| File | Lines |
|---|---|
| TaskProcessor.cpp | 3167 |
| DAVWorker.cpp | 2567 |
| SyncWorker.cpp | 1686 |
| MailStore.cpp | 1412 |
| main.cpp | 1089 |
| MailUtils.cpp | 965 |
| MailProcessor.cpp | 900 |
| Message.cpp | 625 |
| constants.h | 500 |
| Thread.cpp | 476 |
| GoogleContactsWorker.cpp | 286 |
| MetadataWorker.cpp | 233 |
| DeltaStream.cpp | 230 |
| VCard.cpp | 220 |
| NetworkRequestUtils.cpp | 225 |

## 2. Feature scope

Everything the C++ engine does is ported, with these exceptions:

| Item | Treatment | Reason |
|---|---|---|
| TLS below 1.2 (`IMAPAllowInsecureSSL` / `SMTPAllowInsecureSSL` obsolete-TLS ladder) | Dropped. Certificate checks are unchanged. | rustls has no TLS 1.0/1.1. Supporting it would bring OpenSSL back. |
| SASL GSSAPI, Kerberos V4, NTLM, SRP | Dropped | Would need cyrus-sasl or a GSSAPI binding |
| SASL PLAIN, LOGIN, XOAUTH2, CRAM-MD5, DIGEST-MD5 | Implemented in Rust (lettre plus our own code for the last two) | |
| `install-check` mode | Same JSON shape, with checks that make sense without C deps: HTTP reachability, IMAP TLS handshake, SMTP connect. `tidy_check` / `legacy_tls_check` report success. | Most checks existed to diagnose missing shared libraries. |
| `SendFeatureUsageEventTask` | Accepted and completed with no network call | Quotas came from the Mailspring server (D3) |
| `SyncbackMetadataTask` remote phase | Calls `MetadataBackend::push`. The default `LocalOnly` backend is a no-op; the HTTP backend queues pushes it cannot deliver in an outbox. | D3 |
| `MetadataWorker` (delta stream from id server) | Becomes `MetadataBackend::pull`. Not started with `LocalOnly`. With a Tern server: binding check, outbox drain, then the stream from the cursor. | D3 |
| `/api/resolve-dav-hosts` | Replaced by a local DNS SRV lookup (`hickory-resolver`) | D3 |
| libtidy + libxml2 HTML flattening (snippets, search text) | Replaced by an html5ever-based flattener. Snippet text drifts slightly. | No C deps |
| ICU charset detection and CoreFoundation / ICU conversion | `chardetng` + `encoding_rs`. Bodies differ only on mislabelled mail. | No C deps |

Everything else stays, including:
- **Exports and tasks:** bulk `.eml` export (`GetManyRFC2822Task`), contact groups, `EventRSVPTask`.
- **IMAP extensions and fallbacks:** XLIST, COMPRESS=DEFLATE, and the FullHeaders fallback for servers without ENVELOPE.
- **Every provider quirk**, NetEase/Coremail and Hermes included.

## 3. Client contract (frozen, D5)

**Process and CLI**
- **Modes:** `sync`, `test`, `reset`, `migrate`, `install-check`, plus the new `import-legacy`.
- **Options:** `--identity` (JSON or `null`), `--account`, `--orphan`, `--verbose`. Ignore unknown options such as `--info <email>`, which the client passes.
- **Identity line** (stdin after the account line, or `--identity`, which is for tests only): `null` runs with `LocalOnly`. A Tern server object `{"type":"tern-server","version":1,"url","token","userId","serverId"}` selects `HttpMetadataBackend`; an invalid one is logged by field (never the token) and falls back to `LocalOnly`. Any other object is validated as before (`ErrorIdentityMissingFields`, `Invalid Identity JSON`) and then ignored. See `docs/backend-plan.md` 4.1. *(Changed 2026-10-16: the identity was accepted and ignored.)*
- **Environment:** `CONFIG_DIR_PATH` is required. `IDENTITY_SERVER` is accepted and not read.
- **Path check:** release builds exit 2 unless the lowercased argv[0] contains `mailspring`. The harness symlinks around this. Under the rebrand (D11) the check uses the new product name; update the harness symlink at the same time.
- **Account handshake:** without `--account`, print `"\nWaiting for Account JSON:\n"` before reading stdin. The client waits for that first stdout data before writing the account line and the identity line (`../Mailspring/app/src/mailsync-process.ts:220-228`).
- **One-shot output:** one-shot modes print `"\n" + {"error":null|msg}`.
- **`test` mode output:** `{error, error_service, log, account, [error_advice|error_offline]}`. `error` uses the names from `ErrorCodeToTypeMap` (`constants.h:446`) and the client keys on them. `log` is the IMAP/SMTP protocol transcript with AUTH payloads removed. Success exits 0; failure exits 1 with `account: null`.
- **Exit codes:** 0, 1, 2, and 141 when stdin has been closed for more than 30 s. Fatal worker errors abort.
- **stderr** stays empty except for real crashes: the client turns stderr output into the process error.
- **Migrate progress:** the client runs `migrate` once at app start, before any account process (`../Mailspring/app/src/browser/application.ts:99`). It opens a progress window when stdout contains `running migration` or `running vacuum` (`mailsync-process.ts:501-517`).

**stdin commands** (one JSON object per line; unparseable lines are ignored):
- `queue-task {task}`: force `v=0`, run the local phase, then interrupt the foreground IDLE after a 300 ms debounce.
- `cancel-task {taskId}`
- `wake-workers`
- `need-bodies {ids}`
- `sync-calendar`
- `test-crash`, `test-segfault`

**stdout deltas** (`DeltaStream.cpp`)
- **Line format:** one object per line, keys sorted, no spaces: `{"modelClass":…,"modelJSONs":[…],"type":"persist"|"unpersist"}`. serde_json without `preserve_order` sorts keys the same way nlohmann does.
- **Coalescing:**
  - The buffer is keyed by modelClass.
  - A new item merges only into the last buffered item of the same class, and only when the type matches. So persist, unpersist, persist yields three items.
  - JSONs with the same id merge their keys over the earlier entry, so keys present only in an earlier save survive.
- **Flushing:**
  - Classes flush in alphabetical order.
  - Delays: 500 ms for worker stores; 5 ms for the main-thread store; 0 for `ProcessState` (`{accountId,id,connectionError}`), `ProcessAccountSecretsUpdated` and `ProcessIdentityRefreshNeeded`.
  - A later emit with an earlier deadline triggers an immediate flush.
  - Nothing is flushed on exit.
- **Payloads:**
  - `unpersist` carries the full model JSON.
  - `remove<T>(Query)` with no matches emits `modelClass:"none"` with an empty array.
  - `Message` dispatch conditionally adds `body`, `fullSyncComplete`, `rulesReady` and `headersSyncComplete` (`Message.cpp:605-625`).
  - Other delta types: `metadata-expiration`.

**Database**
- **File:** `$CONFIG_DIR_PATH/edgehill.db`, shared by all accounts.
- **Pragmas:** WAL, `page_size=4096`, `cache_size=10000`, `synchronous=NORMAL`, 60 s busy timeout, SQLite temp dir = `CONFIG_DIR_PATH`.
- **Connections:** one connection per worker thread. The client reads the same file read-only with better-sqlite3.
- **Schema:** exactly the C++ V12 final schema, with the same table, column and index names, including the partial-index `WHERE` clauses (the placement `ON CONFLICT` target depends on them).
- **SQLite features:** FTS5 `porter unicode61`, JSON1, `RETURNING` (rusqlite `bundled`).
- **Versioning:** `user_version = 12`, plus `_State` key `engine = rust` (section 9).
- **Derived tables and hooks:**
  - Thread refcounts `_refs`/`_u`
  - `ThreadCategory`, `ThreadCounts`, `ThreadSearch`, `ThreadReference`
  - `ContactSearch`, `EventSearch`
  - `ModelPluginMetadata`, `DetatchedPluginMetadata` (misspelling is real)
  - `MessageFolder`, `MessageOrphan`
  - `test/harness/invariants.py` checks all of them after every scenario.

**Files on disk:**
- Attachments: `$CONFIG_DIR_PATH/files/<id[0:2]>/<id[2:4]>/<id>/<safeFilename>`. The id is lowercased; `safeFilename` replaces each of `[\/:|?*><"#]` with `-` (`MailUtils.cpp:419-433`, `File.cpp:92-95`).
- Log file: `$CONFIG_DIR_PATH/mailsync-<accountId>.log`, 5 MB × 3 rotation (`.log.1`…). Line format `<pid> [YYYY-mm-dd HH:MM:SS.mmm] [<threadName>] [<level>] msg`.
  - Levels: `info`, `warning`, `error`, `critical`.
  - Thread names: `main`, `background`, `foreground`, `calContacts`, `metadata`, `metadataExpiration`, `calendar`.
  - The client reads the last 3000 bytes for crash reports.
  - Lines are flushed about 1 s after activity.
- Logging setup: the file logger exists only in `sync` mode without `--orphan`. Other modes log `"%l: %v"` to stdout.

## 4. Architecture

### 4.1 Process model

Keep the C++ thread layout:
- `main` reads stdin and runs task local phases.
- `background` lists folders, runs scan passes and the orphan sweep, then spawns `foreground`.
- `foreground` does IDLE, body fetches and task remote phases.
- `calContacts` handles DAV and Google People.
- `metadataExpiration` is a local timer.
- `metadata` runs `MetadataBackend::pull`, and only when the backend is not `LocalOnly`.

Each thread owns its own `rusqlite::Connection`. A `Store` is `!Send`, which enforces what
`assertCorrectThread` checks at runtime in C++.

Threads that speak network protocols run a current-thread tokio runtime inside the thread,
because the maintained IMAP and HTTP crates are async. SQLite access stays synchronous.

Shared state is limited to:
- the `DeltaStream` (mutex + condvar + flush thread);
- the worker sleep/wake condvar;
- the foreground IDLE interrupt handle;
- the folder/label cache version counters;
- the OAuth token cache;
- the expiration-worker wake handle.

### 4.2 Workspace layout

The `mailsync` bin crate lives at `apps/engine/` and the `ms-*` library crates at
`crates/ms-*/`, all members of the root Cargo workspace (D12, section 12). P1-H moves the
Python harness from `legacy/mailsync-cpp/test/` to `apps/engine/test/`.

| Crate | Replaces | Key deps |
|---|---|---|
| `ms-core` | `SyncException`, `constants.h` tunables, ids, base58, logging sink, thread names, `Clock` | sha2, bs58, tracing |
| `ms-store` | `MailStore`, `MailStoreTransaction`, `Query`, `DeltaStream`, models, `Placement`, DDL | rusqlite, serde_json |
| `ms-providers` | Every host/greeting/capability quirk check, folder-role tables (`COMMON_FOLDER_NAMES`), OAuth endpoints | none |
| `ms-imap` | `IMAPSession` + libetpan IMAP | own parser and command layer (`docs/spikes/imap.md`), tokio, rustls, rustls-platform-verifier, async-compression |
| `ms-mime` | `MessageParser`, `HTMLRenderer`, header decoding, `Attachment`, `MessageBuilder`, `flattenHTML` | mail-parser, mail-builder, encoding_rs, chardetng, html5ever |
| `ms-smtp` | `SMTPSession` | lettre |
| `ms-net` | `NetworkRequestUtils`, `XOAuth2TokenManager`, Google People client, `MetadataBackend` | reqwest |
| `ms-dav` | `DAVWorker` protocol side, `DavXML`, `DAVUtils`, `VCard`, icalendarlib | reqwest, roxmltree, calcard, hickory-resolver |
| `ms-sync` | `SyncWorker`, `MailProcessor`, `MailboxModel` impls, `MetadataExpirationWorker` | the crates above |
| `ms-tasks` | `TaskProcessor` | the crates above |
| `mailsync` (bin) | `main.cpp`: CLI, modes, stdin loop, thread startup, `test`, `install-check`, `import-legacy` | everything |

### 4.3 Core interfaces

These are written first, in the foundation phase, so parallel packages compile against them.
The signatures are a starting point; adjust them during that phase, not later.

```rust
// ms-core
pub struct SyncError { pub key: String, pub debuginfo: String, pub retryable: bool, pub offline: bool }
// toJSON shape: {"what","key","debuginfo","retryable","offline"} (SyncException.cpp:74-81)

pub trait Clock: Send + Sync { fn now(&self) -> i64; }  // lets tests drive `_sa`, sweeps, intervals

// ms-store
pub trait Model: Serialize + DeserializeOwned {
    const TABLE: &'static str;
    fn columns() -> &'static [&'static str];   // indexed columns besides id/data/accountId/version
    fn bind(&self, stmt: &mut Statement);
    fn before_save(&mut self, store: &Store) -> Result<()> { Ok(()) }
    fn after_save(&mut self, store: &Store) -> Result<()> { Ok(()) }
    fn after_remove(&mut self, store: &Store) -> Result<()> { Ok(()) }
}
// Store::save / remove / find_all<T>(Query) / transaction(name, |tx| …) / placement helpers

// ms-providers
pub struct ServerProfile { /* named quirk flags, see 4.4 */ }
pub fn resolve(info: &ConnectionInfo) -> ServerProfile;  // host, greeting, ID response, capabilities

// ms-imap: workers depend on this trait, not on async-imap, so the spike outcome is contained
pub trait ImapSession {
    fn select(&mut self, path: &str) -> Result<SelectInfo>;
    fn status(&mut self, path: &str) -> Result<FolderStatus>;
    fn fetch_by_uid(&mut self, path: &str, kind: FetchKind, uids: &UidSet) -> Result<FetchResult>;
    fn sync_by_uid(&mut self, path: &str, kind: FetchKind, uids: &UidSet, modseq: u64) -> Result<SyncResult>;
    fn take_vanished(&mut self, path: &str) -> UidSet;
    fn idle(&mut self, path: &str, interrupt: &IdleInterrupt) -> Result<()>;
    // store_flags, store_labels, move_uids, copy_uids, expunge(_uids), append, create/rename/delete folder,
    // list_folders, fetch_message_bytes, find_uids_of_recent_header_message_id
}

// ms-sync
pub trait MailboxModel {  // FolderModel and GmailLabelModel
    fn classify_folders(&self, listed: &[ListedFolder]) -> (Vec<Folder>, Vec<Label>);
    fn on_placement_upserted(&self, store: &Store, msg: &mut Message, folder: &Folder) -> Result<()>;
    fn sent_placement_folder(&self, store: &Store) -> Result<Folder>;
}

// ms-net
pub trait MetadataBackend: Send {
    fn push(&self, model_id: &str, plugin_id: &str, value: &Value, version: i64) -> Result<()>;
    fn pull(&self, cursor: Option<&str>, sink: &mut dyn FnMut(MetadataDelta)) -> Result<String>;
}
pub struct LocalOnly;  // default: push is a no-op, pull is never started
```

### 4.4 Provider profiles

There are two layers, so that "is Gmail" and "has MOVE" are never the same kind of check:

1. **Capabilities**, parsed from CAPABILITY and ENABLE, drive the generic code paths: CONDSTORE vs
   QRESYNC vs plain scans, MOVE vs COPY+`\Deleted`+EXPUNGE, UIDPLUS, IDLE, NAMESPACE, XLIST,
   COMPRESS.
2. **`ServerProfile`** is plain data, resolved once per connection. It can override capabilities
   and names each quirk as a field.

Each provider is one file under `ms-providers/src/providers/`, holding a `detect` function and a
profile. Providers are matched in order, with a generic default last. Each profile flag comes with
a fake-server personality in `test/fakeimap/personalities.py` that exercises it.

| Provider / server | Profile content | C++ evidence |
|---|---|---|
| Gmail (detected by `X-GM-EXT-1`) | `mailbox_model = GmailLabels`; XLIST off; LITERAL+ search; login-error text mapping; OAuth (Google); People API contacts; fixed CalDAV URL `apidata.googleusercontent.com/caldav/v2/<email>` | `SyncWorker.cpp:961-964`, `MailProcessor.cpp:299-304`, `MCIMAPSession.cpp:1074-1103, 4500-4515`, `DAVWorker.cpp:445-460` |
| O365 / Exchange | OAuth (Microsoft, `IMAP.AccessAsUser.All` + `SMTP.Send`); `SMTPUsername` for shared mailboxes; `createHelperFolders=false` for shared mailboxes; LIST NO treated as connection error; `late_sent_copies`; Outlook XOAUTH2 variant for `smtp-mail.outlook.com`, decided by the SMTP host (`smtp_uses_outlook_xoauth2`), not the IMAP profile | `SyncWorker.cpp:860-864, 765-769`, `MailUtils.cpp:894-897`, `MCSMTPSession.cpp:91, 638-642`, `NetworkRequestUtils.cpp:58-100` |
| iCloud (`imap.mail.me.com`) | `disable_qresync` | `MailUtils.cpp:882`, `SyncWorker.cpp:402`, `Account.cpp:125` |
| Yahoo | honour MESSAGELIMIT (RFC 9738); XYMHIGHESTMODSEQ; SEARCH charset NULL. The multi-message COPYUID distrust that Yahoo motivated applies to every server (`trust_copyuid_pairing` defaults to false) | `TaskProcessor.cpp:129-216`, `SyncWorker.cpp:1212-1224, 1294-1298`, `MCIMAPSession.cpp:3705` |
| ProtonMail Bridge | `skip_all_mail_folder`; `merge_duplicate_list_entries` | `SyncWorker.cpp:498-507, 811-834` |
| Dovecot | `noop_before_selected_status`; BYE on UIDVALIDITY change | `SyncWorker.cpp:1184-1194, 1348-1361`, `MCIMAPSession.cpp:1467` |
| Cyrus / Fastmail | `vanished_upper_bound_uidnext` (cyrus-imapd #6071) | `SyncWorker.cpp:1394-1400` |
| Courier | force IDLE and NAMESPACE on. `reselect_after_append` is on for every server, as in C++ (`TaskProcessor.cpp:2438-2441`) | `MCIMAPSession.cpp:862-867`, `TaskProcessor.cpp` send path |
| Coremail / NetEase | ID before SELECT; `C`-prefixed tags (detected from the greeting); `status_uidnext_unreliable` | `MailUtils.cpp:849-856`, `MCIMAPSession.cpp:857-861, 4819`, libetpan `mailimap.c:2461, 2550` |
| hMailServer, home.pl | `status_uidnext_unreliable`; XLIST off for IdeaImapServer | `SyncWorker.cpp:447-460`, `MCIMAPSession.cpp:4504` |
| Yandex | XLIST off. `tolerate_idle_disconnect` is on for every server, as in C++ (`SyncWorker.cpp:302-313`) | `MCIMAPSession.cpp:4505`, `SyncWorker.cpp:310-313` |
| Hermes | ignore the NAMESPACE capability | `MCIMAPSession.cpp:4547` |
| DavMail, Zoho | `merge_duplicate_list_entries` | `SyncWorker.cpp:811-834` |
| DAV servers (Yahoo, Fastmail, iCloud, Nextcloud, SOGo, Zimbra, Posteo, Synology, DAViCal, Bedework, Radicale, GMX, Baikal, Xandikos) | `DavProfile` flags, taken from the comments at `DAVWorker.cpp:650, 835, 1020-1023, 1360, 1782-1801, 2112, 2335` | |

Gmail is the one provider that changes the data model rather than toggling behaviour: labels
instead of folders, exactly one placement (All Mail, Spam or Trash), and All Mail placement on
send. All of that sits behind `MailboxModel`.

### 4.5 Crates

Health was checked on crates.io and GitHub in Oct 2026. Licences are MIT, Apache, ISC or MPL,
all compatible with the GPLv3 output.

| Concern | Crate | Notes |
|---|---|---|
| IMAP | own parser and command layer on tokio + rustls | The section 10 spike ruled out `async-imap` 0.12 / `imap-proto` 0.17 (no VANISHED in command results, no MESSAGELIMIT code, strict ENVELOPE, fixed tag prefix; `docs/spikes/imap.md`). They remain dev-dependencies for the spike example only. |
| MIME parse | `mail-parser` 0.11 (Stalwart) | Lenient. Exposes the part tree needed for partID/uniqueID numbering. `mailparse` (used by Delta Chat) is the fallback. |
| MIME build | `mail-builder` 1.0 (Stalwart) | |
| SMTP | `lettre` 0.11 | rustls; raw-envelope send for per-recipient sends; XOAUTH2/PLAIN/LOGIN |
| iCal + vCard | `calcard` 0.3 (Stalwart) | Parse and build both formats |
| CalDAV/CardDAV client | own client on `reqwest` + `roxmltree` | `libdav` 0.11 was evaluated and not adopted (`docs/spikes/dav.md`). |
| SQLite | `rusqlite` 0.40 with `bundled` | |
| HTTP | `reqwest` 0.13 | |
| TLS | `rustls` + `rustls-platform-verifier` | OS trust store |
| Charsets | `encoding_rs`, `chardetng` | |
| HTML | `html5ever`; `html2text` as a reference | |
| DNS SRV | `hickory-resolver` | |
| Ids | `sha2`, `bs58` | bs58's last release was 2024, but it is a small, finished crate |
| Modified UTF-7 | write it (~80 lines) | `utf7-imap` has been unmaintained since 2022 |
| Logging | `tracing` + our own size-rotating writer | Needed to reproduce the exact line format and the `.log.N` naming |

Reference code to read, not depend on: Delta Chat core (IMAP sync on async-imap against real
servers) and pimalaya `email-lib` (IMAP/SMTP/OAuth glue).

## 5. Message identity (D4)

The C++ id inputs depend on mailcore's ENVELOPE decoding. A fresh DB means we no longer have to
match it, but the id still has to be deterministic from the message alone. That keeps an id
stable across moves, restarts and rebuilds, and across devices once a sync backend exists.

- **Format (unchanged):** `base58(sha256(src)[0..30])`, Bitcoin alphabet. The client doesn't parse ids.
- **Message `src`:** `accountId + "-" + date + subject + "-" + participants + "-" + messageId`.
  - `date`: the Date header as epoch seconds, falling back to INTERNALDATE.
  - `subject`: RFC 2047-decoded, NFC-normalised UTF-8.
  - `participants`: the To+Cc addr-specs, lowercased, sorted bytewise, joined with `""`.
    Bcc is left out (C++ hashes To+Cc+Bcc): our Sent copy keeps Bcc and the other copies
    (the provider's Sent copy, a Bcc'd self's Inbox copy) don't, so they got different ids
    and the late-copy cleanup never matched them (fix-backlog F1). A Bcc'd message already
    synced keeps its old id until its copy is fetched again (UIDVALIDITY change, account
    reset), which reads as a delete + create.
  - `messageId`: the Message-ID without angle brackets, or `""` when absent.
- **Dropped from C++:** scheme 0 (local-timezone dates) is gone.
- **No date:** keep the C++ fallback `folderPath:uid`, but log it. Such ids are not stable across moves.
- **Freezing it:** a golden test file of `(raw headers → id)` pairs locks the algorithm. Changing
  the decoder must never silently change ids.
- **Other ids** keep their C++ definitions (`MailUtils.cpp:524-611`, `Thread.cpp:23`):
  - folder `accountId:rawPath`, with the path still modified UTF-7 and `INBOX` normalised;
  - thread `t:<first message id>`;
  - file `msgId:accountId:partID:uniqueID`;
  - draft, event, calendar and contact keys.
- **Random ids:** 40 alphanumeric characters (`idRandomlyGenerated`); deletion placeholders are `deleted-<random>`.
- **Missing Message-ID:** `hMsgId` stores a generated `uuid@hostname`, but the id hash uses `""`.

## 6. Behaviour notes that are easy to miss

These are behaviours a reader of the C++ would most likely get wrong. Each work package
should start by reading the cited code.

**Sync** (`SyncWorker.cpp`, `MailProcessor.cpp`)
- **Request kinds** (`MailUtils.cpp:925-939`):
  - Heavy fetch: `ENVELOPE`, `BODY.PEEK[HEADER.FIELDS (References)]`, INTERNALDATE and FLAGS; plus X-GM-LABELS, X-GM-THRID and X-GM-MSGID on Gmail.
  - Light fetch: FLAGS, plus X-GM-LABELS on Gmail.
  - A message missing any requested attribute, or with UID 0, is dropped.
- **Header precedence:** INTERNALDATE overrides the ENVELOPE date for `receivedDate`. `_data.date` is `date == -1 ? receivedDate : date`.
- **Login sequence** (`MCIMAPSession.cpp:876-1195, 4824`):
  1. CAPABILITY.
  2. LOGIN or `AUTHENTICATE XOAUTH2`.
  3. COMPRESS if advertised.
  4. `ENABLE QRESYNC`, or else `ENABLE CONDSTORE`.
  5. ID (Coremail).
  6. NAMESPACE, or else `LIST "" ""` for the delimiter.
  - SELECT is a plain SELECT.
  - STATUS asks for `UNSEEN MESSAGES RECENT UIDNEXT UIDVALIDITY` plus HIGHESTMODSEQ when available.
  - IDLE times out at 28 min.
- **VANISHED:** collect untagged VANISHED from every response, not only from the FETCH that asked for it (`MCIMAPSession.cpp:4575-4645`). libetpan only surfaces the first VANISHED per response (`SyncWorker.cpp:1441-1443`).
- **INBOX naming:** any case variant of `inbox` becomes `INBOX`. If none is listed, the first `\Inbox`-flagged folder is used; failing that, `LIST "" INBOX` (`MCIMAPSession.cpp:1579, 1717-1737`).
- **Role detection** (`MailUtils.cpp:313-417`, `SyncWorker.cpp:996-1057`):
  - Role order: `all, sent, drafts, spam, important, starred, archive, inbox, trash, snoozed`.
  - For each role, flags are checked before names.
  - Name matching strips the namespace prefix and delimiter, lowercases, handles `[Mailspring]/X`, then decodes modified UTF-7 and looks the name up in `COMMON_FOLDER_NAMES`.
  - `\NoSelect` folders are dropped.
  - `Mailspring/Snoozed` is created unless `createHelperFolders` is false.
  - `COMMON_FOLDER_NAMES` also has `[gmail]/important`, `[google mail]/important`,
    `[gmail]/starred` and `[google mail]/starred`, which C++ lacks, so Gmail labels listed
    without their special-use flag still get the role (fix-backlog F2, forum topic 227).
- **Gmail labels** (`MailUtils::labelForXGMLabelName`, `MailUtils.cpp:447-461`; fix-backlog F2,
  scenario `gmail-important`):
  - An X-GM-LABELS user label resolves by exact path. A system name (`\Important`) resolves
    by role (singular or plural), then by a `[Gmail]/` or `[Google Mail]/` path naming it.
    C++ matched every path first, then `\Name` against bare paths and roles in one pass, so
    a user label named "Important" took `\Important` (a chip, and removing it stored the
    wrong label).
  - A ChangeLabelsTask stores `[Gmail]/Important` as `\Important` even when the label has
    no role. C++ stored the path, which failed with ErrorStore.
  - When a folder sync creates or removes a label or gives one a role, the threads whose
    copies carry an X-GM-LABELS name that now resolves differently (another label, none, or
    a changed path or role) are recounted from their messages. C++ left them as counted,
    so Important shown in IMAP again only reached old threads after Rebuild Cache
    (Mailspring #443).
- **Constants** (`SyncWorker.cpp:29-58, 1496-1506`):
  - Header fetch chunks: 750 on first sync, 1024 otherwise. Below 5000 messages a single range sweep is used.
  - Modseq truncation: when the gap exceeds 4000, fetch only the last 12000 UIDs.
  - Scan intervals: shallow every 2 min, deep every 10 min, CONDSTORE gap scan every 24 h.
  - Orphan wait: 24 h (env `ORPHAN_SWEEP_MAX_WAIT`).
  - Bodies: the last 3 months, at most 30 per pass, none in Spam or Trash; MessageBody rows are purged after 14 days.
- **Orphan sweep bound** (`folderCoveredAt`): see the repo CLAUDE.md. It is the subtlest rule in sync.

**Bodies** (`MailProcessor.cpp:384-551`, `Vendor/mailcore2/src/core/renderer/MCHTMLRenderer.cpp`)
- **Rendering by MIME type:**
  - `multipart/alternative`: prefer HTML, then plain, then the first part. Append any `text/calendar` alternative (Exchange invites).
  - `multipart/mixed`: concatenate the parts. If any direct child is HTML, render every text part as HTML.
  - `multipart/related`: render part 0; parts 1..n become inline attachments.
  - `multipart/signed` is treated as mixed. `message/rfc822` recurses into its main part.
- **What counts as body text:** `text/plain` or `text/html` that is not an attachment, and is not a named part after the first rendered part unless marked inline.
- **Plain text inside HTML:** lines starting with `>` become `<blockquote type="cite">`; other lines are escaped and joined with `<br/>`.
- **No HTML part at all:** the body is stored with `plaintext=true`.
- **HTML bodies are stored as-is**, not tidied.
- **Part numbering:**
  - partIDs: a top-level single part is `1`; multipart children are `idx+1`, joined with `.`; `message/rfc822` adds a `.1` prefix.
  - uniqueIDs: BFS from the root, numbering single parts `0, 1, …` (`MCAbstractPart.cpp:311-337`, `MCMessageParser.cpp:224-312`).
- **Files:**
  - A part with a Content-ID referenced as `cid:` in the HTML becomes inline. Duplicate partIDs are skipped.
  - When a file has no contentId and the body contains `cid:<filename>`, contentId is set to the filename.
  - Unnamed parts get default names (`File.cpp:40-65`).
  - Thread `attachmentCount` counts files with no contentId or larger than 12 KB.
- **Snippet:** the flattened HTML (links get ` (url)`; cited blockquotes are kept with `> ` prefixes, because `flattenHTML()` is `flattenHTMLAndShowBlockquote(true)`, `MCString.cpp:2162`), whitespace collapsed, cut to 400 UTF-16 units. Search text is cut to 5000.
- **Also extracted:** `List-Unsubscribe`, `List-Unsubscribe-Post`, and importance (Importance → X-Priority → X-MSMail-Priority).
- **Mail-sourced contacts** (`MailProcessor.cpp:823-890`, `contactKeyForEmail` at `MailUtils.cpp:238-266`):
  - Created from participants, and they power composer autocomplete.
  - Skipped when the address is longer than 40 characters, starts with an `unworthyPrefixes` entry, or contains `@noreply` / `@notifications` / `noreply@`.

**Tasks** (`TaskProcessor.cpp`)
- **Lifecycle:**
  - `local` → `remote` → `complete` / `cancelled`.
  - No automatic retries: any failure ends as `complete` with `error`. A SQLite error leaves the task as it was.
  - On launch, `local` tasks are deleted.
  - Pruning keeps the last 100 finished tasks (slop 10) and emits unpersist deltas.
  - The foreground loop re-queries `status="remote" AND rowid>?`, so tasks queued by tasks (metadata from a send) run in the same pass.
- **Drafts are local only:** `SyncbackDraftTask` has no remote phase.
- **Moves** (`:134-302, 453-594`):
  - The remote phase uses UID MOVE, or COPY + `\Deleted` + UID EXPUNGE / EXPUNGE without MOVE.
  - New UIDs are re-verified by fetching headers, because Yahoo permutes COPYUID.
  - Without MOVE, flags are re-applied on the destination.
  - `undoPlacements` / `restorePlacements` pairing follows the rules in the repo CLAUDE.md.
- **Send** (`:2085-2517`):
  - **Single run:** `_performRemoteRan` is saved first so a send never runs twice.
  - **Stopped mid-send:** `_smtpDone` is saved on the task once SMTP accepts the message. A
    relaunch that finds the task still `remote` with it set removes the draft's server
    copies and its row; without it (stopped before or during SMTP) the draft is kept. C++
    returns early either way and leaves the draft, which the user then sees next to the
    sent message (fix-backlog F1). The Sent copy is not placed again: a provider copy syncs
    as usual, and on a provider that files none a send stopped before the APPEND has no
    Sent copy. The draft's metadata (open and link tracking) is not attached either.
  - **Body:**
    - HTML is sent as `multipart/alternative`, with the text part generated by flattening.
    - `contentId` files go under `multipart/related`; other files are attachments, read from the files dir.
    - Files are looked for where the client writes them (`files/xx/yy/<id>/<safeDisplayName>`), then at the C++ `safeFilename` path. A file that can't be read fails the task with `send-failed` before SMTP runs; C++ sent an empty part in its place (`MCAttachment` on a missing path). Downloaded inline parts are written to the client path too.
  - **Headers:**
    - `In-Reply-To` and `References` are both set to the single `replyToHeaderMessageId`. A forward sets only `References`.
    - Priority headers come from `hImportance`.
    - `User-Agent: Mailspring`.
  - **Bcc:** stripped from the wire copy but kept in the Sent copy. It is not part of the
    message id (section 5), so the two copies get the same id.
  - **Multisend** (`perRecipientBodies` with a required `self` entry): one SMTP send per recipient, with RCPT set to that one address and the full headers. Failure keys are `send-partially-failed` / `send-failed`.
  - **After a successful send:**
    1. Delete the draft's server copies.
    2. Poll Sent for the provider's copy with `findUIDsOfRecentHeaderMessageID` (re-SELECT + fetch the last ~16 by sequence). Delays are `{0,1,1,2}` s, or `{0,1,1,2,2,3,3}` once `LS_SERVER_SAVES_SENT` is set.
    3. Multisend: delete every copy found (on Gmail, from All Mail too). Otherwise use the server copy, or APPEND the `self` MIME with `\Seen` plus the thread's Gmail labels.
    4. Gmail: place the message in All Mail.
    5. Re-select, fetch headers and flags, and insert using the local MIME as the body.
       Without UIDPLUS the appended copy is found by Message-ID (one match only); C++
       skipped the insert and `_asc` when APPEND returned no UID.
    6. When we appended, write `_asc`, valid for `SENT_COPY_CLEANUP_WINDOW` (7 days; C++
       30 min).
    7. Run `draft.metadata` as SyncbackMetadataTasks, where a `thread:` prefix targets the thread.
  - **Late copies:** `removeLateSentCopies` runs every background pass for Sent-role folders.
    It starts from the messages in Sent dated within the window whose `_asc` names that
    folder, and while the `_asc` UID is still there removes every other Sent copy of the
    message and of any message with the same `headerMessageId`; a row that held one of
    those copies and is left with none is removed at once. C++ starts from messages with more than one Sent copy, so a late
    copy that hashed to another id (Bcc, a rewritten header) or came after 30 min stayed
    (fix-backlog F1).
- **RSVP** (`:2891-3167`): validate the iTIP REPLY/COUNTER (RFC 5546) and send `multipart/alternative` (text/plain + `text/calendar; method=`) to the organizer. No Sent copy.
- **Bulk export** (`:2633-2889`):
  - Exports 50 messages per chunk, with the UID as cursor.
  - Filenames: `NNNNN - subject - YYYY-MM-DD.eml`.
  - After each chunk it saves progress JSON and checks for cancellation; it sleeps 1 s between chunks.
- **Error JSON:**
  - `{"what","key","debuginfo","retryable","offline"}`.
  - Connection errors are retryable and offline. Parse and Fetch errors are retryable.
  - Unknown exceptions become `unhandled-exception`, with `what` cut to 1024 characters of printable ASCII.

**Auth `test` mode** (`main.cpp:301-460`)
- **IMAP:** connect and list folders. Require a folder with the `inbox` role, or `all` on `imap.gmail.com`.
- **SMTP:** when `settings.smtp_verification == "login"`, only log in. Otherwise MAIL FROM / RCPT TO self, then send a real "Mailspring SMTP Test Email" to self.
- **Error advice:** TLS hints come from `MailUtils::tlsFailureAdvice` (`MailUtils.cpp:789`).

**Metadata (local)**
- `SyncbackMetadataTask` local phase: `upsertMetadata` on any model via `findGeneric`, then write `modelMetadataNewVersion`.
- `MailModel::afterSave` rewrites `ModelPluginMetadata` rows when versions change, and wakes the expiration worker.
- `MetadataExpirationWorker` emits `metadata-expiration` deltas. snooze, send-later and send-reminders in the client depend on these.
- Metadata for a model that doesn't exist yet waits in `DetatchedPluginMetadata`.

**DAV** (`DAVWorker.cpp`)
- **Discovery:**
  - SRV lookup, then `.well-known`, then current-user-principal, then the home-set, then the collections.
  - Gmail uses a fixed CalDAV URL, and contacts go through the People API.
- **Sync:**
  - A ctag check first.
  - RFC 6578 sync-token, with an ETag/REPORT fallback.
  - Calendar window: 12 months back to 18 months ahead.
  - RFC 6585 429 / Retry-After backoff.
- **Auth:** OAuth Bearer when a refresh token exists, otherwise Basic with the IMAP credentials.
- **Writes:**
  - PUT/DELETE with If-Match; a 412 becomes `etag-conflict`. This covers contact PUT and event DELETE as well as event PUT; C++ maps 412 only for event PUT (`DAVWorker.cpp:2469`), and `dav-etag-conflict` expects all three. A 412 on DELETE is re-checked with a getetag PROPFIND, and a 404/410 there counts as already gone.
  - A new CardDAV contact or group card is PUT to `<addressbook>/<uid>.vcf`, and the card's UID equals the Contact id. C++ POSTs to the collection (`DAVWorker::writeAndResyncContact`) and gives a new group a random UID (`performLocalSyncbackContactGroup`).
  - Create re-checks with a REPORT read-back.
  - A new event is PUT with `If-None-Match: *`, as a new card is, so a resource not yet synced is never overwritten (RFC 4791 §5.3.2). C++ sends no precondition on creates (`DAVWorker.cpp:2420-2424`).
  - 404/410 on delete counts as success.
  - A refused remote phase does not leave the local phase's edit behind (C++ does): a SyncbackEvent re-reads the stored resource, or removes an event that was never written; DestroyContact unhides the contacts it did not delete; a CardDAV DestroyContactGroup rebuilds the group from its card. The server's ctag does not change on a refusal, so no sync would repair these rows.
- **Google People:** a connection with `metadata.deleted == true` is a tombstone and removes the local contact. C++ checks a top-level `deleted` the API never sends, so it saves a blank contact (`GoogleContactsWorker::run`). An expired sync token clears the token and fails the pass; the full re-list runs on the next contacts pass, as in C++.
- **Contact groups:** removing a group leaves `gis` on its members and removing a contact leaves its join rows, as in C++ (`ContactGroup::afterRemove`, `Contact::afterRemove`). The `contacts` invariant accepts these leftovers.
- **Schedule:** calendars every 15 min, contacts every 75 min, the first run 15 s after start. `sync-calendar` runs calendars only, never two at once.
- **Event fields:**
  - The raw ICS is stored in `_data.ics`, plus UID, RECURRENCE-ID, STATUS, DTSTART and the search fields.
  - `rs`/`re` are range bounds. `re` is RRULE UNTIL when present, else `20370000T000000`.
  - The client expands recurrences.
  - Fixing TZID handling with `chrono-tz` is allowed and encouraged.

## 7. Test strategy

- **Primary gate:** `MAILSYNC_BIN=target/release/mailsync python3 -m pytest apps/engine/test` (see
  `test/README.md`).
  - The harness is black-box and runs against the fake IMAP/SMTP server, Dovecot and Cyrus (Docker).
  - `invariants.py` runs after every scenario.
- **Harness changes owned by this port** (the tests package, P1-H):
  - Make IMAP tag regexes tag-agnostic, e.g. `^\d+ IDLE$` → `^\S+ IDLE$`, and the `sent`/`recv` hook patterns.
  - Replace the `migration-*` scenarios (C++ schema upgrades) with `fresh-db-replaces-legacy` and `import-legacy-carries-metadata`.
  - Add scenarios for local metadata expiration, Google People (against a fake People API), and DAV (against Radicale and Baikal in Docker, under `test/servers/`).
- **What the harness relies on** (`test/harness/mailsync.py`, `test/README.md`):
  - **Launch:**
    - The binary comes from `MAILSYNC_BIN`, else `--mailsync`. It is symlinked as `<work>/mailspring-bin/mailsync` to pass the argv[0] check.
    - Env: `CONFIG_DIR_PATH`, `IDENTITY_SERVER=http://127.0.0.1:9/unreachable`, plus the scenario's `env:`.
    - It always runs `--mode migrate` first and requires exit 0 and `"error":null`.
    - Then `--mode sync --verbose --identity null --account <json>`, with the account passed as an argument rather than on stdin.
    - Account id `c0ffee-harness` must give `startDelay` 0 (`Account.cpp:32-39`).
    - Stop sequence: close stdin, SIGTERM, then SIGKILL after 5 s.
  - **Logs:** it tails the log file every 0.1 s, including across `.log.1` rotation, matching `^(\d+) \[ts\] \[thread\] \[level\] msg`. Stdout lines without `modelClass` are recorded as noise.
  - **Quiescence**, all of which must hold:
    - every Folder has `localStatus.busy == false` and `syncedMinUID <= 1` (read from the DB);
    - the last `background` line is exactly `Sync loop complete.` and is at least 2 s old;
    - the last `foreground` `sent` line is an IDLE command, or its last line is `Connection does not support idling…` (`Idling on folder` when not verbose);
    - no delta has arrived for 2 s.
  - **`sync: pass` step:** sends `wake-workers` and waits for ``Marking all folders as `busy` ``.
  - **`force_scans`:** writes `localStatus.lastDeep/lastShallow/lastCleanup = 0` into the live DB, so the engine must re-read Folder `localStatus` on every pass.
  - **Task completion:** detected through the `Task.status` column (`complete` / `cancelled`).
  - **Endpoint overrides** the engine reads in place of its compiled-in hosts (`test/README.md`):
    - `MAILSYNC_GOOGLE_TOKEN_URL` (and `MAILSYNC_MICROSOFT_TOKEN_URL`) replace the OAuth token endpoints; `MAILSYNC_GOOGLE_PEOPLE_ROOT` replaces `https://people.googleapis.com/v1/` (ms-net `Endpoints::from_env`).
    - `MAILSYNC_GOOGLE_CALDAV_ROOT` replaces the Gmail calendar host `apidata.googleusercontent.com` (ms-dav `gmail_caldav_host`).
    - `MAILSYNC_CALDAV_HOST` / `MAILSYNC_CARDDAV_HOST` (`host[:port]`) replace the DNS SRV lookup; discovery then runs from `.well-known` (ms-dav `env_host_override`).
  - **Hook regexes** match the command text the engine sends: `^UID FETCH 1:\*`, `BODY\.PEEK\[\]$`, `^UID STORE .*Deleted`, `^STATUS "`, `^UID (MOVE|COPY)`, `^UID EXPUNGE`, `^LIST`. Matching mailcore's command shapes keeps the fake server on the parsing paths that have been checked against Dovecot.
  - **Running:** `pip install -r test/requirements.txt`, then `python3 -m pytest test` (16 in parallel, ~6 min), narrowed with `-k`, `--servers fake` or `-n 0`. `python3 test/run.py <yaml> --server kind:profile --keep` runs one scenario. Artifacts go to `test/runs/session-<pid>/`.
- **A/B:** `test/tools/ab.py` runs both binaries on the same scenario. Use it to diff deltas and DB
  state (ids will differ by design, so compare by `headerMessageId`).
- **Unit tests per crate:**
  - id golden file;
  - MIME rendering golden files (raw `.eml` → body, snippet, files);
  - provider resolution table;
  - delta coalescing;
  - placement helpers checked against invariants ported from `invariants.py`.
- **Real accounts before shipping:** Gmail, O365, iCloud, Yahoo, Fastmail, plus a Dovecot host. Diff
  `test` mode output against C++ for each.

## 8. Work breakdown

Each package lists its C++ sources, its output and its gate. Packages within a phase are
independent and can run in parallel. A phase starts when the packages it depends on have passed
their gates.

**P0 Foundation (serial, one agent).**
- Workspace and CI (`cargo fmt`, `clippy -D warnings`, `cargo test`).
- Root `Cargo.toml` workspace (`apps/engine`, `apps/backend` once it exists, `crates/*`), `cargo deny` licence allowlist, and `apps/engine/CLAUDE.md` carrying over the comment and logging conventions from `legacy/mailsync-cpp/CLAUDE.md`.
- All of the 4.3 interfaces, with stub implementations.
- `ms-core`: SyncError, id functions with the golden file, base58, constants, log sink and line format, Clock.
- `mailsync` bin: CLI parsing, the account/identity handshake, `migrate` creating the fresh V12 schema (section 9), and the stdin loop with no-op handlers.
- Gate: the harness's migrate step passes and sync starts and idles without crashing (no IMAP yet).

**P1 Libraries (parallel).**
- **P1-S store** (`MailStore.*`, `MailStoreTransaction.*`, `Query.*`, `DeltaStream.*`, `Models/*`, `Placement.*`, `constants.h`):
  - all models and hooks, thread refcounting, ThreadCategory/ThreadCounts/ThreadSearch;
  - placement helpers and `refreshMessageFromPlacements`;
  - transactions with delta batching; DeltaStream coalescing and flush timing; `reset` mode.
  - Gate: unit tests with ported invariants, delta coalescing tests, `reset` output.
- **P1-I IMAP** (`MCIMAPSession.cpp` behaviour notes in section 6; `MailUtils::configureSessionForAccount`):
  - the `ImapSession` impl on async-imap (section 10 spike), TLS/STARTTLS, LOGIN/XOAUTH2, verbose `sent`/`recv` logging with AUTH redacted, modified UTF-7;
  - `ms-providers` with every profile in 4.4.
  - Gate: `test/conformance` probes plus a scripted session against fake, Dovecot and Cyrus.
- **P1-M MIME** (`MCMessageParser`, `MCHTMLRenderer`, `MCAttachment`, `MCMessageBuilder`, `flattenHTML`):
  - parse to rendered body + files + headers;
  - partID/uniqueID numbering, snippet and search text;
  - outgoing builder (alternative/related/mixed, priority headers, Bcc handling).
  - Gate: golden `.eml` corpus.
- **P1-N net** (`NetworkRequestUtils.*`, `XOAuth2TokenManager.*`, `GoogleContactsWorker.*` client side):
  - HTTP helpers;
  - OAuth refresh for Google/Microsoft with `ProcessAccountSecretsUpdated` on rotation, a token cache with a 60 s margin, string or missing `expires_in`;
  - `MetadataBackend` with `LocalOnly`;
  - People API client.
  - Gate: unit tests against a mock HTTP server.
- **P1-T SMTP** (`MCSMTPSession` notes in section 2 and section 6): `ms-smtp` on lettre, CRAM-MD5 and DIGEST-MD5, error mapping to the `ErrorCodeToTypeMap` names, and the self-test send. Gate: harness SMTP sink tests.
- **P1-D DAV library** (`DavXML.*`, `DAVUtils.*`, `VCard.*`, `Event.cpp` ICS fields): XML helpers, vCard and iCal field extraction on calcard, `rs`/`re`, and the SRV discovery client. Gate: unit tests on captured responses.
- **P1-H harness:** the changes listed in section 7, plus Radicale/Baikal server images and a fake People API. Gate: the C++ binary still passes the unchanged scenarios.

**P2 Sync (depends on P1-S, P1-I, P1-M).**
- **P2-B background worker:**
  - folder listing, roles, `MailboxModel` impls, STATUS/CONDSTORE/QRESYNC/plain passes, range diff;
  - modseq truncation, MESSAGELIMIT, UIDVALIDITY reset and relink, orphan sweep, `removeLateSentCopies` hook;
  - header ingestion, threading via ThreadReference or X-GM-THRID, mail contacts.
  - Gate: `baseline-initial-sync`, `condstore-*`, `qresync-*`, `uidvalidity-*`, `orphan-sweep-*`, `remote-*`, `sparse-uid-space`, `gmail-labels`, `proton-*`, `o365-duplicate-sent-copies`, `folder-roles-by-localized-name`, `gateway-duplicate-list-entries`, `list-refused-transiently`, `modseq-truncation`, `messagelimit-partial-fetch`, `netease-coremail-id-before-select`, `status-uidnext-zero-new-mail`, `plain-expunge-found-by-deep-scan`, `two-folders-identical-messages`, `self-addressed-inbox-and-sent`.
- **P2-F foreground worker:**
  - IDLE loop with interrupt, `need-bodies`, body selection and fetch, rules-ready;
  - `ProcessState` connection errors; the worker error policy (abort on non-retryable, `--sleeping` on retryable).
  - Gate: `rules-ready-*`, `new-mail-during-idle`, `connection-dropped-*`, `heavy-fetch-truncation-backlog`, `mid-pass-foreground-tombstone`, `qresync-vanished-during-body-fetch`.

**P3 Tasks (depends on P2).**
- **P3-T mail tasks:**
  - the queue lifecycle;
  - Unread, Starred, ChangeFolder with undo, Labels, SyncbackDraft, DestroyDraft, SyncbackCategory, DestroyCategory, ChangeRoleMapping, ExpungeAllInFolder;
  - GetMessageRFC2822, GetManyRFC2822, local SyncbackMetadata, SendFeatureUsageEvent as a no-op.
  - Gate: `client-task-*`, `move-*`, `undo-*`, `trash-*`, `mark-read-*`, `star-*`, `flag-change-*`, `synced-draft-destroy*`, `remote-move-while-task-in-flight`.
- **P3-S send (also needs P1-T):** SendDraft (single and multisend), Sent reconciliation, Gmail All Mail placement, late-copy cleanup, and `EventRSVPTask`. Gate: `send-draft`, `sent-copy-*`, `gmail-send-and-labels`, `rules-ready-self-addressed-send`.
- **P3-A modes:** `test` with the exact output and the protocol log, plus `install-check`. Gate: output diffed against C++ for the harness accounts and for a manual Gmail/O365 run.

**P4 Contacts and metadata (depends on P3).**
- **P4-G Google People:** contact and contact-group sync and tasks, on the `calContacts` thread. Gate: fake People API scenarios.
- **P4-M metadata:** the expiration worker; `DetatchedPluginMetadata` attach; `MetadataBackend` wiring, so a future backend only implements the trait. Gate: new snooze and send-later expiration scenarios.

**P5 DAV (depends on P1-D, P3).**
- CalDAV/CardDAV workers, event and contact write tasks, contact groups, and DAV profiles.
- Gate: new Radicale/Baikal scenarios for initial sync, incremental sync, create/update/delete and etag conflict.

**P6 Ship (depends on everything).**
- `import-legacy` (section 9).
- macOS signing/notarization and Linux packaging.
- A client setting that selects the engine binary.
- `ab.py` over the full suite; dogfooding on real accounts.

## 9. Fresh database and legacy import

- `migrate` mode, run once by the client at startup:
  - If `edgehill.db` exists and `_State.engine` is not `rust`, move `edgehill.db`, `-wal` and `-shm` to `edgehill.legacy.db*`. Print `Running Migration` so the client shows its progress window.
  - Then create the V12 schema directly (one DDL set, not a V1→V12 replay), set `user_version = 12` and `_State.engine = rust`.
  - Delete `$CONFIG_DIR_PATH/files/` contents only after the new DB is created: bodies and attachments are re-downloaded.
    Local drafts' files exist nowhere else, so before emptying `files/` migrate moves their
    directories to `files.legacy/` (`ms_store::legacy_import::set_aside_draft_files`);
    import-legacy, a later process, copies them back and removes `files.legacy/`.
- `import-legacy` mode (P6) reads `edgehill.legacy.db` read-only and imports:
  - `ModelPluginMetadata` for messages and threads, re-keyed by `headerMessageId` into
    `DetatchedPluginMetadata` pending rows (`objectId` `<hMsgId>` for a message,
    `t:<hMsgId>` + U+001F + legacy thread id for each message of a thread) that attach on
    the message's first save, or at once to messages already synced (this keeps snoozes,
    reminders and send-later);
  - local drafts (Message rows with a UID-0 placement and none on the server, or before V10
    `remoteUID = 0`), under their legacy ids, plus `MessageBody`, their File rows, files and
    metadata; the placement's folder is created by path if sync has not listed it yet. The
    legacy `threadId` names nothing in the new database (D4): a reply or forward draft joins
    the thread of the message its `rthMsgId` (else `fwdMsgId`) names, at once if that
    message is synced, otherwise through a pending `DraftThread` row (`t:<hMsgId>` + U+001F +
    draft id) when it is first saved; other drafts get no thread;
  - `cursor-*`. Other `_State` keys are not carried: `gsynctoken-*` would make the first
    People pass incremental over contacts that were not imported.
  - Contacts, events and mail are not imported; they resync from the server.
  - Trigger: the client runs it after `migrate` on every launch, before any account
    process, with no `--account`. It is a no-op without a legacy DB or once `_State`
    `legacy-import` is set (written in the import's transaction), so a draft the user
    deleted or sent is never imported twice. After the commit `edgehill.legacy.db*` is
    renamed to `edgehill.legacy-imported.db*` (an older copy is replaced), so a new database
    created without a new legacy move (the user deleted `edgehill.db*` to resync) imports
    nothing. Output is the one-shot `{"error":…}`; a failure exits 1 and the client launches
    anyway (`MailsyncProcess.migrate` in apps/desktop/src/shared/engine-process.ts runs it
    when the Rust engine is selected).
- Future schema changes are Rust-owned migrations starting from this V12 baseline.

## 10. Risks and spikes

- **IMAP spike (first task of P1-I):**
  - Drive async-imap through the conformance probes and a scripted CONDSTORE/QRESYNC/IDLE session against fake, Dovecot and Cyrus.
  - The questions: access to unsolicited responses (VANISHED, EXPUNGE, FETCH during IDLE), MESSAGELIMIT response codes, lenient ENVELOPE parsing, IDLE interrupt from another thread, `C`-prefixed tags, and COMPRESS.
  - If async-imap fails on any of these, keep imap-proto as the parser and write the command layer behind the same `ImapSession` trait (2–4k lines).
- **IMAP leniency:** libetpan has absorbed years of server misbehaviour. Mitigation: verbose protocol logs, a parser that skips unparseable untagged responses instead of failing the command, and dogfooding on real servers.
- **Snippet and body drift** from replacing tidy, ICU and CoreFoundation. It is visible to users but loses no data. Diff a few thousand real messages before P6.
- **Harness coupling to log text:** about 40 scenario regexes match specific log lines (`grep -h log test/scenarios/*.yaml`). Keep those strings in one Rust module (`ms-core::logmsg`) so wording changes are deliberate.
- **DAV has no tests today.** P1-H must deliver the DAV servers before P5 starts.

## 11. Windows (later)

No Windows builds until parity. Rust removes most of the cost the C++ build has (vcpkg,
OpenSSL, SASL), so the later work is mostly packaging. To keep it cheap, from P0 on:
- add `cargo check --target x86_64-pc-windows-msvc` to CI;
- no Unix-only APIs outside a `platform` module (signals, `exit(141)` stdin handling, file permissions);
- build paths with `Path`, never by joining strings with `/`;
- rely on `rustls-platform-verifier` for the trust store.

The current C++ Windows build is 32-bit (`Win32`); the Rust one targets x86_64 and arm64 only.

## 12. Monorepo and licensing

### Layout

Local checkout: `~/Developer/OpenSource/Ternmail/ternmail`, which maps to GitHub `ternmail/ternmail`.

```
ternmail/
  Cargo.toml          workspace: apps/engine, apps/backend, crates/* (D12)
  apps/
    desktop/          Electron client (TS), imported from Mailspring with history
    engine/           mailsync bin crate: CLI, modes, stdin loop, thread startup; harness in test/
    backend/          self-hostable metadata/tracking/sharing server, Rust (D3; docs/backend-plan.md)
  crates/             Rust libraries: the engine's ms-* crates, and ms-wire shared by engine
                      and backend
  packages/           TS libraries, e.g. protocol types generated from the engine's delta/task
                      structs so the client and engine can't drift
  legacy/
    mailsync-cpp/     the C++ engine, imported with history; reference and A/B baseline until
                      parity, then deleted
  docs/
  LICENSE.md          GPL-3.0 text
  NOTICE.md           upstream attribution (below)
```

The libraries are split by language so each tool globs its own folder: Cargo uses `crates/*`
and npm uses `packages/*`. Deployables live in `apps/`. Both upstream repos were imported with
`git subtree add`, so authorship and copyright lines are intact. Mailspring's `mailsync` git
submodule was removed after the import.

### GPL-3.0 obligations

Both upstream repos are received under GPL-3.0-or-later. The Rust engine is a translation of the
C++ engine, so it is a derivative work too. The whole monorepo is distributed as GPL-3.0-only.
This is not legal advice.

- Keep the upstream copyright and licence notices. Add a top-level NOTICE crediting Foundry 376
  and the Mailspring / Mailspring-Sync contributors, with links to the upstream repos.
- Mark modified files with a prominent notice that they were changed, and when (GPLv3 §5a). A
  NOTICE entry for the forked directories plus git history covers this in practice. The Rust
  files get a header naming the C++ file they derive from.
- Distribute binaries only with the corresponding source available (§6). A public repo with
  tagged releases that match the binaries is enough.
- Add no further restrictions. In particular, no proprietary licence terms on anything that
  links into the engine or the client.
- Dependencies must be GPLv3-compatible. MIT, Apache-2.0, ISC, BSD and MPL-2.0 all are. Add
  `cargo deny` with an allowlist to CI in P0, and the JS equivalent for `apps/desktop`.
- **`apps/backend`:** a separate program that talks over HTTP, written from scratch. It is
  GPL-3.0-only like the rest of the repo (decided 2026-10-16, replacing MIT; backend-plan O6)
  and could move to MIT later by a deliberate decision. It must not depend on any engine crate
  (`ms-core`, `ms-store`, ...); shared protocol types live in `crates/ms-wire`.
- **Crates or packages published from `crates/` or `packages/`:** these can carry a permissive licence only if they contain
  no code derived from the GPL sources.

### Trademark and naming

"Mailspring" is Foundry 376's product name, and the Mailspring-Sync README forbids using the
project names to endorse or promote derived products. The new product needs its own name.
Remove Mailspring branding (app name, icons, About, update URLs, `*.getmailspring.com`) from
`apps/desktop`. Keep attribution in NOTICE and the About screen as credit, not as branding.

Engine strings that look like branding but are functional need a decision:
- `User-Agent: Mailspring` on outgoing mail: rename.
- The `Mailspring/Snoozed` server folder and the `[Mailspring]/X` role convention exist on
  users' servers already. Keep recognising them, and choose whether new installs create
  `Tern/Snoozed` instead.
- `edgehill.db` and the `mailsync` binary name are internal and can stay.
