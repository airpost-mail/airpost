# Tech debt carried over from Mailspring

Inherited debt, code smells and dated choices worth fixing while the thin-client refactor is
underway. Found on 2026-10-16 by read-only audit of `docs/tech-debt` (same tree as `main`), then
checked item by item. Work that `docs/fix-backlog.md`,
`docs/architecture.md` or `docs/plugins-inventory.md` already schedules is left out. "Stage N"
means a stage of the desktop client refactor, now finished. Desktop paths are relative to `apps/desktop/`,
Rust paths to the repo root. Each item says whether it was verified in the tree or is inferred.

Items are grouped by when to do them, and ranked by impact over size inside each group.

| # | Item | Area | Impact | Size | When |
|---|---|---|---|---|---|
| **Do now** | | | | | |
| TD1 | Attachments open with no quarantine or Mark-of-the-Web | security | high | ~1 day | now |
| TD2 | Remote-image blocking is a regex; message iframe has no CSP | security | high | ~200 TS | now |
| TD3 | Remote images on by default, trackers stripped only from own mail, Gravatar always loads | privacy | high | ~1 day | now |
| TD4 | Onboarding and engine fall back to cleartext IMAP/SMTP | security | high | ~1 day | now, before stage 12 |
| TD5 | "Reset Accounts and Settings" runs under a live engine and re-imports Mailspring | ux | high | ~80 TS | now |
| TD6 | No schema migration path for Rust-written databases (done) | engine | high | ~150 Rust | now, before stage 7 |
| TD7 | ThreadReference lookup on every message insert is a table scan (done) | engine | high | ~40 Rust | now, after TD6 |
| TD8 | Query planner has no statistics (no ANALYZE / optimize) | engine | medium | ~60 Rust | now or stage 7 |
| TD9 | `npm ci` runs `npm install` and `npm dedupe` in `app/` | build | medium | 0.5–1 day | now, before stage 2 |
| TD10 | Forms in mail can navigate the reading pane | security | medium | ~40 TS | now |
| TD11 | App CSP duplicated and weak; signature preview unsanitised | security | medium | ~40 TS | now |
| TD12 | Mail, raw source and logs in fixed `/tmp` names; engine files at default umask (done) | security | medium | ~80 TS, ~40 Rust | now; engine half before stage 11 |
| TD13 | Outdated vendored parsers run on attachments (done) | security | medium | ~1 day | now |
| TD14 | CI: no dependency updates, no npm licence gate, mutable action tags with signing secrets (done but the PR builds and weekly specs) | build | medium | 1–2 days | now |
| TD15 | Huge emails parsed in full twice per render despite the clip | perf | medium | ~100 TS | now |
| TD16 | Localisation: placeholder off-by-one, drifting keys, no plurals | i18n | medium | <30 lines + CI | now, before stage 4 |
| TD17 | Nine help links point at a 15-line README | ux | medium | ~1 day | now |
| TD18 | No update check at all | ux | medium | ~80 TS | before first public release |
| TD19 | Accessibility: no dialog semantics, focus handling, live regions; low contrast; no reduced motion | ux | medium | ~3 days | now |
| TD20 | Body-fetch placeholder rows never retried | engine | medium | ~40 Rust | now, before stage 9 |
| TD21 | Threading ignores In-Reply-To and never merges | engine | medium | S / M | In-Reply-To now |
| TD22 | Desktop specs need a release engine build | build | medium | S | now |
| TD23 | No permission handlers; `--harmony` V8 flag (done) | security | low | ~30 lines | now |
| TD24 | Abandoned single-use dependencies | deps | low | 1–2 days | now |
| TD25 | First-run tutorial and settings advertise features that can't work | ux | low | 0.5 day | now |
| **Fold into a stage** | | | | | |
| TD26 | Engine protocol types hand-copied in desktop and web (done) | architecture | high | ~400 lines + CI | before stage 1 |
| TD27 | TypeScript non-strict, lint stack EOL with most rules off | build | medium | 2–3 days | before stage 1 |
| TD28 | No Electron fuses (done) | security | high | ~50 lines | after stage 2 + database-agent swap |
| TD29 | No plugin trust model before `Engine` export and service plugins | security | medium | ~200 TS + docs | before stage 2 |
| TD30 | Plugins can't be disabled from the UI; dead and placeholder plugins | ux | medium | ~250 TS | stage 2 |
| TD31 | `webviewTag` on every window for a component nothing renders (done) | security | low | ~−200 lines | stage 2 |
| TD32 | Reset / Rebuild Cache deletes local-only plugin metadata | engine | high | ~350 Rust | before stage 4 |
| TD33 | No outbox; failed silent sends pop a composer | ux | medium | ~10 TS now; ~500 later | silent fix now; queue in stages 4/13 |
| TD34 | Message bodies render inside the Node-enabled renderer | security | high | ~700 TS | before stage 5 (plugin API 1.27) |
| TD35 | `@electron/remote` is the renderer-to-main channel | architecture | high | M–L, ~60 files | stages 3 and 6, then any order |
| TD36 | Store graph held together by the `mailspring-exports` barrel and require order | architecture | high | M | before stage 6 |
| TD37 | `AppEnv` is an undeclared plugin API; stage 6 changes `mailsyncBridge` under plugins | architecture | medium | S | stage 6 |
| TD38 | Deltas fan out unbatched through four event systems | architecture | low | S / M | stage 6; rx-lite after 8 |
| TD39 | Folder identity is a path hash; rename is delete + create (done for renames) | engine | medium | ~1 week | stage 3, before 14 |
| TD40 | MessageStore reloads every body on each delta and expands every unread message (done) | perf | medium | ~150 TS | before stage 8 flip |
| TD41 | Every thread row subscribes to ComponentRegistry three times (done) | perf | medium | ~80 TS | before stage 8 |
| TD42 | ThreadSearch is one append-only FTS row per thread | engine | high | ~1–1.5 weeks | before stage 9 |
| TD43 | Transient errors in a task's remote phase revert the user's action | engine | high | ~300 Rust | before stages 11/14 |
| TD44 | Any non-SyncError stops the account for good | engine | medium | ~80 Rust | before or with stage 11 |
| TD45 | No fuzzing of untrusted-input parsers; a panic stops the account | engine | medium | 2–3 days | before stage 11 |
| TD46 | Monthly blocking VACUUM, WAL never truncated | engine | low | ~80 Rust | stage 11 |
| TD47 | Provider presets: dead entries, three copies, unbounded MX lookup | ux | low | 1–2 days | stage 12 |
| TD48 | Three 12/24-hour rules; reply attribution line is English 12-hour | i18n | medium | ~150 TS | before stage 13 |
| TD49 | Very large sync and send functions | engine | low | M, spread | stage 13 / with TD39 |
| TD50 | EventSearch and ContactSearch updated by an UNINDEXED column (done) | engine | medium | ~120 Rust | before stage 18 |
| TD51 | Three timezone databases, two copies of ical.js | deps | low | M, mostly free | after stage 18 |
| **Later** | | | | | |
| TD52 | React 17 with APIs React 19 removes | deps | medium | ~1 week + M | codemod now; React 18 after stage 8 |
| TD53 | Composer on a 2019 Slate fork pinned to personal GitHub commits | deps | medium | S vendor; L replace | vendor now; replace after stage 13 |
| TD54 | Four packaging paths, dead grunt and C++ leftovers | build | medium | tiny + 1–2 weeks | dead bits now; builder after stages 2/10 |
| TD55 | STATUS on every folder every 2 minutes, IDLE only on Inbox | engine | medium | ~1 week | after stage 10 |
| TD56 | Error reporting would leak a stable device id and unscrubbed data (done) | privacy | low | ~100 TS | before any DSN is set |
| TD57 | A LESS compiler kept for v1 third-party stylesheets | deps | low | ~1 day | at plugin API 2.0 |

## Do now

These are cheap, independent of the refactor's stages, or block something it does soon.

### TD1. Attachments open with no quarantine or Mark-of-the-Web

Done (roadmap/security, df38be698). `extra.plist` sets `LSFileQuarantineEnabled`. The new
`src/shared/attachment-safety.ts` tags a file as downloaded, with `/usr/bin/xattr -w com.apple.quarantine
0081;<hex epoch>;Tern;<uuid>` via `execFile` on macOS (in `files/` an existing attribute is
kept, so an approved file isn't re-checked; saved copies always get a fresh value, since a file
overwritten in place keeps the old file's xattrs) and a `Zone.Identifier` stream (`ZoneId=3`) on Windows. It runs
on the attachment before `shell.openPath`, on the copy after Save / Save All and after Save Image
from a message, and on the source at drag start. A failure is logged and doesn't stop the open.
Opening a file whose last extension (case-insensitive, trailing dots/spaces and bidi/zero-width
characters ignored, so `invoice.pdf.exe` and `invoice<U+202E>fdp.exe` count) is on the
platform's denylist asks first through a main-process dialog (`attachment-safety:confirm-open`,
Cancel by default); a decoy extension or hidden characters are named in the prompt, which shows
the name with them stripped. The denylists cover Outlook's Level 1 list (minus certificates and
data files), NTLM-leaking types (library-ms, search-ms, theme), rdp, py* and extra disk images;
on Linux they include Windows types for wine, and a file starting with MZ, ELF or `#!` asks too
(review fixes, 83a65a63f). Save-as now resolves when the write
finishes and reports write errors. Not done: the drag-out drop target is a fresh copy Chromium
writes from the `DownloadURL`, so on Windows it likely has no Zone.Identifier, and on macOS it
relies on the main process's `LSFileQuarantineEnabled` (untested); switching to
`webContents.startDrag` would let the OS copy carry the source's tag. Helper processes have their
own Info.plist without the key, so renderer writes outside these paths aren't quarantined.
Deferred to stage 16: the engine setting the xattr when it writes `files/`.

Verified. Double-clicking an attachment calls `shell.openPath` on the raw file
(`src/flux/stores/attachment-store.ts:168-175`) with no prompt or extension check. Save-as is a
plain stream copy (`:177-184`) and drag-out sets `DownloadURL` from the raw `file://` path
(`src/components/attachment-items.tsx:140-147`). No code in the desktop, the Rust engine or the
C++ engine sets `com.apple.quarantine` or `Zone.Identifier`. `build/resources/mac/extra.plist`
has no `LSFileQuarantineEnabled`, and the app isn't sandboxed, so macOS doesn't quarantine on its
own.

- Cost: a `.command`, `.terminal`, `.jar`, `.scpt` or an unsigned app from a zip runs on
  double-click, skipping Gatekeeper's first-launch check in most cases. On Windows, `.exe`, `.lnk`
  and `.js` skip SmartScreen, and Office files skip Protected View (which keys on MotW). Mail.app,
  Outlook and Thunderbird all guard this path.
- Fix: on macOS, add `LSFileQuarantineEnabled = true` to the plist; it covers files the app writes
  with no xattr code. The engine writes `files/` itself, so either the key applies to the engine
  binary too or the engine sets the xattr when it writes (the `xattr` crate is MIT/Apache). On
  Windows, write a `Zone.Identifier` ADS (`ZoneId=3`) before `openPath` and after save and
  drag-out. Add a confirm dialog for an executable-extension denylist.
- Size: plist key plus ~150 TS and ~80 Rust. Move the engine half into whichever stage makes
  `files.read` engine-owned (stage 16).

### TD2. Remote-image blocking is a regex; the message iframe has no CSP

Done (roadmap/security, f3bd26b43). The reading pane, clipped-message, show-original and print
documents start with a CSP meta from `src/shared/content-security-policy.ts` (`img-src`/`font-src` get
`https: http:` only when allowed; `form-action 'none'`). Inline cid images are served as
`tern-file:` from `<config>/files` with traversal and symlink escape rejected. MessageBodyProcessor
returns `remoteImages` from the new `MessageViewExtension.allowsRemoteImages` hook (the autoload
plugin answers; the setting decides if nothing does) and a change remounts the iframe, since a
document keeps every policy across `doc.open()`. The regex now only drives the banner. Quick
preview follows the global setting only, as it has no sender; print allows remote images only if
every printed message does.

Review fixes (roadmap/security, a7e9107e6). CSP 'self' and `file:` match file://host/... (a UNC
path on Windows) from a file: document, so `webRequest.onBeforeRequest` now cancels hosted file:
URLs on the default session, and the quick-preview partition only allows file: inside its bundle
and `<config>/files`. Copy Image and dragging an inline image into a draft work with `tern-file:`.
The banner counts http(s) URLs only (any `url(`/`image-set(`, not cid:/data:). Still open: the
composer's quoted body renders in the main window without the message CSP, so its remote images
load even when blocking is on.

Verified (regex run in node). "Block remote images" rewrites HTML with the pattern in
`features/message-autoload-images/lib/autoload-images-store.ts:25-35`. It misses
`background:#fff url(https://…)`, `background:no-repeat url(…)`, CSS escapes like `\75rl(`, and
`image-set("https://…" 1x)`. The sanitizer keeps the `style` attribute and `<style>`
(`src/services/sanitize-transformer.ts:93`, `:223`) and only strips `@import` (`:250-273`).
`email-frame.tsx:113-118` `doc.write`s the body into an `about:blank` iframe with no CSP, so it
inherits the app policy `default-src *; img-src * data: mailspring: file:`
(`static/index.html:6`, `src/main/main.js:466-467`). `font-src` falls back to `default-src *`,
so remote `@font-face` is a second path. cid images are rewritten to `file://`
(`message-item-body.tsx:161`). Quick preview of attachments sets `img-src 'self' data: https:
http:` and never checks the setting (`src/quickpreview/index.ts:25-33`).

- Cost: tracking pixels fire with blocking on, which is the privacy promise users rely on.
  Because `file:` is allowed, CSS `url(file://host/share/x.png)` can probably trigger an SMB
  connection and leak an NTLM hash on Windows (inferred, not run).
- Fix: write a `<meta http-equiv="Content-Security-Policy">` first in the iframe:
  `default-src 'none'; style-src 'unsafe-inline'; img-src data: <cid-scheme>; font-src data:`
  when blocking, plus `https:` when allowed. Serve cid images through the custom scheme, limited
  to `<config>/files`, so `file:` leaves the email CSP. Apply the same CSP to the print,
  show-original and clipped windows and to quick preview. Keep the regex only to decide whether
  to show the "images blocked" banner.
- Size: ~200 TS. Display stays in the desktop, so no stage depends on it.

### TD3. Remote images on by default; trackers stripped only from your own mail; Gravatar always

Done (roadmap/security, ca461e035). `remove-tracking-pixels` now strips, from all mail and not
only `isFromMe`, remote images that match a tracker rule or are hidden (`display:none`,
`visibility:hidden`, `hidden`, zero-sized or 1x1). The rules are MailTrackerBlocker's
(BSD-3-Clause, 468 regexes over 320 vendors, credited in NOTICE.md) plus the old list's uncovered
entries; Mailspring read receipts are stripped whoever's they are. `core.reading.autoloadImages`
defaults to `false` (explicit `true` kept, per-sender allowlist unchanged), and the new
`core.reading.loadGravatar` (off) gates `ContactProfilePhoto`'s Gravatar lookup. Review fixes
(b0442b65a): rules run on each `src`/`srcset` URL of `<img>`, `<source>` and `<input type=image>`
(first 2 KB, `.+` capped at 256), not the whole tag, so crafted attributes can't hang the renderer;
anything but cid:/data:/app schemes counts as remote; opacity:0, max-width/height and negative
sizes count as hidden. New drafts use only vendor rules, so path rules like `/open.gif` don't
remove forwarded images. With autoload off the composer shows uneditable blocks without remote
image URLs (the user's signature images excepted); the sent HTML keeps them. Not covered: CSS
`background-image` and `background=` trackers, class-based, off-screen, 2x2 and CSS-escaped
hiding (the message CSP still blocks all of these when images are off), and the signature
editor's user-chosen Gravatar photo. The per-sender allowlist keys on the unauthenticated From
address; checking Authentication-Results needs headers the desktop doesn't have.

Verified. `core.reading.autoloadImages` defaults to `true` (`src/window/config-schema.ts:146-150`). The
`remove-tracking-pixels` plugin applies its 25-entry, circa-2017 list (Sidekick, RelateIQ, Banana
Tag, LaunchBit, Mandrill; `features/remove-tracking-pixels/lib/main.ts:9-135`) only when
`message.isFromMe()` (`:158-169`), and has no 1x1 or hidden-image rule. Spam is the one exception
(`autoload-images-store.ts:47`). Every profile photo also loads
`gravatar.com/avatar/<sha256(email)>` whatever the setting (`src/components/contact-profile-photo.tsx:18-26`).

- Cost: every opened newsletter sends a read receipt and leaks IP and user agent. Gravatar sees
  which senders the user reads and when, and the hashes can be reversed with a dictionary. The
  plugin's name promises protection it doesn't give. Thunderbird, Proton and Apple Mail block or
  proxy by default.
- Fix: strip the tracker list and 1x1 / `display:none` images from all mail. Default
  `autoloadImages` to `false` for new profiles, keeping the per-sender allowlist that
  `message-autoload-images` already has. Put Gravatar behind a setting that defaults off, or tie
  it to `autoloadImages`. Refresh the list from a maintained source.
- Size: ~1 day. Not scheduled: stage 17 covers outgoing tracking, and the refactor only drops the
  composer-side pixel removal into `draft.create`.

### TD4. Onboarding and the engine fall back to cleartext IMAP/SMTP

Done (roadmap/security, a7ff50c51, 1f20eb6dd). `Security::from_setting` accepts exactly `"SSL / TLS"`,
`"STARTTLS"` and `"none"` (what the desktop's menu stores); `Account::validate` refuses any
other, empty or missing value with `Account has an invalid imap_security: … (expected …)`,
which `--mode sync`/`test` print as their one-shot error and `account.add`/`account.test`
return as INVALID_PARAMS, before any connection. The accessors fall back to TLS on an
unvalidated account. Harness scenario `account-security-refused` (all three transports):
`none` syncs from the plaintext fake server, an empty `imap_security` and a `"None"`
`smtp_security` are refused with no connection to their servers. Onboarding: ports 143 and 25
pick STARTTLS; discovery (Mailcore, provider table, autoconfig `plain`) never picks `none`;
choosing `none` for a non-loopback host blocks Connect until "Send password unencrypted" is
checked (stored as `settings.<imap|smtp>_allow_cleartext`), with `127.0.0.1`, `localhost` and
`::1` exempt. The provider table's 135 `none` entries (plus soverin.net, which had no
`imap_security` on port 143) now say STARTTLS on 143/587 (port 25 moved to 587). Hosts weren't
probed: one without STARTTLS now fails onboarding's connection test, and the user can pick
`none` with the checkbox. Older autoconfig code stored `"None"` for a `plain` server; such an
account is now refused with the error above until the user picks a security again (no
migration).

Review fixes (roadmap/security, cce371ecf, 57b8ec13f, 860e40e0f). Lowercase `none` stored before this fix was
often discovery's pick, not the user's (the 135 provider entries, templates with a port and no
security, the 143/25 port switch), so the engine now refuses `none` for a non-loopback host
unless `settings.<proto>_allow_cleartext` holds that host (`Account has imap_security "none"
for "<host>" without imap_allow_cleartext …`); the accessors fall back to TLS the same way.
Such an account stops on launch and goes to Reconnect, where an `imap` account gets the
checkbox and a provider account is rediscovered (now STARTTLS). The confirmation stores the
host it was given for, so changing the host asks again (a bare `true` doesn't count). On
Reconnect, a stored `"None"`, empty or missing security is replaced with what discovery picks
for the port, so the menu shows what the account holds. The web client's account form asks the
same way (`account-draft.ts`). Harness: `account-security-refused` adds a remote-host `none`
with a bare `true`; `account_json` stores the host consent for its `none` accounts.

Verified. Typing port 143 sets IMAP security to `none`, port 25 does the same for SMTP
(`features/onboarding/lib/page-account-settings-imap.tsx:90-98`). Discovery picks `none`
for any IMAP port other than 993 and any SMTP port other than 465/587 (`onboarding-helpers.ts:185`,
`:194`). 135 of the 1064 entries in `mailspring-provider-settings.json` ship `none` (105
plala/wind `.jp`, plus nifty.com, goo.jp, terra.es, telefonica.net, versatel.de, tiscali.cz,
peoplepc.com and others). In the engine, `Security::from_setting`
(`crates/tern-core/src/account.rs:25-31`) maps any unknown, empty or missing value to `Clear`, so it
fails open, and `Account::valid()` doesn't check it. `crates/tern-smtp/src/session.rs:142` then
greets with no TLS. The UI shows no warning.

- Cost: passwords and mail in clear for users who type 143, for those domains, and for any
  account JSON with a bad value. Stage 12 ports discovery into `tern-providers` and would copy the
  table and defaults as-is.
- Fix: engine accepts only an explicit `"none"` as `Clear` and rejects unknown values with an
  error (existing `none` values weren't always the user's choice; see the review fixes above). UI: port 143 defaults to STARTTLS,
  and saving `none` needs a "send password unencrypted" checkbox, except for `127.0.0.1` (Proton
  Bridge). Data: move the 135 entries to STARTTLS where the host supports it, drop the rest.
- Size: ~20 Rust, ~60 TS and a data pass, ~1 day. Before stage 12, so the cleaned table is what
  gets ported.

### TD5. "Reset Accounts and Settings" runs under a live engine and re-imports Mailspring

Done (roadmap/security, 782eaac27). Preferences sends `application:reset-profile`; main confirms
with a detail text that names the directory and lists what goes, stops `config.json` writes,
destroys the windows, stops the engine through `EngineSupervisor.shutdown()` (serve's
`engine.shutdown`, killed after the grace period), clears the default session's storage, then
deletes everything in the config dir except the running instance's `Singleton*` lock
(`src/main/profile-reset.ts`). Nothing imports a Mailspring profile on its own any more (Import
from Mailspring is an action in onboarding and Preferences > Accounts,
`src/main/profile/mailspring-import.ts`), so no marker is needed. The dir is whatever
`main.js` resolved (`Tern`, `Tern-dev`, `--config-dir-path`, snap); the filesystem root, the home
folder and `appData` are refused. Entries that can't be deleted are listed and the app relaunches
anyway. The supervisor and `engine.shutdown`
named under Size already exist, so nothing waits on stages 6 or 10.

Review fixes (roadmap/security, a5d05e112). Reset only empties a dir carrying `.tern-profile`
(`src/main/profile-marker.ts`), which `main.js` writes for the default dir and for a
`--config-dir-path` it created or found empty; an existing custom dir from before this can't be
reset. Paths are compared after `realpath`, case-folded on macOS and Windows, and the dir may not
equal or contain home or `appData`. Windows' `lockfile` is kept, and Chromium storage that fails to
delete after `clearStorageData` is logged, not listed. From the confirm on, `WindowManager` opens
no windows and `QuitGate` defers launches (Dock, second instance, mailto:) into the relaunch, for
the database reset too.

Verified, except the engine-still-running part, which is inferred from the code path.
`features/preferences/lib/tabs/preferences-general.tsx:29-49` asks only "Are you sure?",
then runs `fs.rm(configDir, {recursive})` from the renderer and relaunches. Nothing tells main or
the engine first; compare `src/main/application.ts:349`, which at least destroys windows before
deleting the database. On relaunch `config.json` is missing, `main.js:56-60` points the importer
at `~/Library/Application Support/Mailspring`, and `shouldImportMailspringProfile()`
(`src/main/mailspring-profile-import.ts:44-46`) returns true with no marker, so the Mailspring
profile, accounts and database are copied in again (`main.js:404`).

- Cost: for anyone who also has Mailspring installed, reset does nothing visible. Deleting while
  `serve` writes its WAL, `run/engine.json` and logs can leave a half-deleted directory.
- Fix: a main-process `application:reset-profile` command: destroy windows, stop the engine,
  delete, write a marker that suppresses the import (or ask "Import from Mailspring again?"),
  relaunch. Dialog detail text lists what gets deleted.
- Size: ~80 TS. Route through the supervisor once stage 6 lands and `engine.shutdown` once stage
  10 adds it. Plan overlap: stage 10 only covers `account.reset` and `engine.resetDatabase`.

### TD6. No schema migration path for Rust-written databases

Done (pre-launch engine pass, 62210758e). `create_v12` writes `user_version` 12 and `migrate`
then applies `tern_store::schema::MIGRATIONS` (V13 onwards), each in an IMMEDIATE transaction
that re-reads `user_version`, so two engines on one profile don't apply a step twice. An
existing database being upgraded prints `Running Migration`, so the client shows its progress
window. A version above `SCHEMA_VERSION` is refused. `v12_schema.json` stays the C++ base,
`tests/fixtures/migrations.json` lists what the migrations add, and `tests/schema.rs` upgrades a
V12 profile written through the store and checks that its rows survive.
`schema_check.py check` expects V12 plus the migrations.

Verified. `crates/tern-store/src/migrate.rs:57-77` either creates V12 on an empty database or
errors on a Rust database below version 12, and silently accepts a version above
`SCHEMA_VERSION` (`:78-79`), so an older build would run against a schema it doesn't know.
`crates/tern-store/tests/schema.rs:4-6` requires the schema to match the C++ V12 byte for byte.
The port's rule is that future changes are Rust-owned migrations from V12, but no
code does it and no stage schedules it.

- Cost: every schema fix in this list (TD7, TD42, TD46, TD50) either waits or forces users to
  resync. The trial profile and early users already have V12 Tern databases.
- Fix: an ordered list of Tern migrations (V13, V14, …) run in `migrate` inside a transaction,
  bumping `user_version`. Refuse versions above the build's. Keep the V12 fixture as the base and
  assert base plus migrations. The `constants.h` comparison already skips when the file is
  missing (`tests/schema.rs:103-109`); delete it at stage 10. Additive indexes are fine for the
  desktop's read-only SQLite access before then.
- Size: ~150 Rust plus tests.

### TD7. ThreadReference lookup on every message insert is a table scan

Done (pre-launch engine pass, 62210758e). Migration V13 creates `ThreadReferenceLookup ON
ThreadReference (accountId, headerMessageId)` and deletes references whose thread is gone;
`Thread::after_remove` deletes a thread's references. The lookup's plan is now `SEARCH
ThreadReference USING INDEX ThreadReferenceLookup` (asserted in `tests/schema.rs`). A/B
snapshots against the C++ engine differ on `ThreadReference` after a thread is removed.

Verified (EXPLAIN on the V12 schema). Non-Gmail accounts find a new message's thread with
`WHERE ThreadReference.accountId = ? AND headerMessageId IN (…)`
(`crates/tern-store/src/search.rs:131-157`, called inside the insert transaction at
`crates/tern-sync/src/mail_processor.rs:212`). The only index is the primary key
`(threadId, accountId, headerMessageId)` (`crates/tern-store/src/schema.rs:43`), and the plan is
`SCAN ThreadReference`. Thread removal never deletes its references (`Thread::after_remove`,
`thread.rs` ~625-634; only account reset does, `schema.rs:92`), so the table only grows. The join
on `Thread` drops orphans, so they cost time, not wrong matches. Inherited from `constants.h:138`.
Gmail is unaffected (it uses `ThreadGmailLookup`).

- Cost: initial sync of a large IMAP or Exchange account is roughly O(n²), and each new message
  gets slower as the mailbox ages. Whether this blocks other accounts' writes is inferred.
- Fix: `CREATE INDEX ThreadReferenceLookup ON ThreadReference(accountId, headerMessageId)` as the
  first TD6 migration, and delete a thread's references in `Thread::after_remove`.
- Size: ~40 Rust plus fixture update. Before stage 7, so the parity fixture and its EXPLAIN
  checks include it.

### TD8. Query planner has no statistics

Verified (EXPLAIN on the V12 schema). Nothing in `crates/tern-store/src` or `apps/engine/src` runs
`ANALYZE` or `PRAGMA optimize`. Without `sqlite_stat1`, the `threads.get` message query
(`crates/tern-store/src/read.rs:259`, `WHERE threadId = ? AND accountId = ? ORDER BY date`) uses
`MessageListDateIndex (accountId=?)` plus a temp B-tree instead of `MessageListThreadIndex`.

- Cost: opening one thread from the CLI, tern-mcp or web walks every message in the account. The
  stage 7 query compiler will generate more queries with the same exposure.
- Fix: `PRAGMA optimize` after `migrate` and from the background cleanup pass (or a bounded
  `ANALYZE` with `analysis_limit`). Use `INDEXED BY` or reorder terms in hand-written read
  queries. Add EXPLAIN QUERY PLAN assertions for hot queries next to the stage 7 parity tests.
- Size: ~60 Rust plus plan tests.

### TD9. `npm ci` runs `npm install` and `npm dedupe` in `app/`

Done (flatten stage 1). `app/` is gone: `apps/desktop` has one `package.json` and one
`package-lock.json`, and `npm ci` installs exactly that lock. The postinstall
(`apps/desktop/scripts/postinstall.js`) only rebuilds the native runtime deps for Electron
(`npm rebuild better-sqlite3 windows-focus-assist` with the `npm_config_*` Electron env), then runs
the nanosleep and ABI checks. No install or dedupe runs after the lock is applied. The yarn
`packageManager` and `resolutions` were removed earlier; `overrides` holds the pins.

### TD10. Forms in mail can navigate the reading pane

Done (roadmap/security, f3bd26b43). `EventedIFrame` cancels every submit in the frame, confirms,
then opens the GET URL with its fields (POST: the action URL only) through `openLink`. The message
CSP's `form-action 'none'` covers the gap before its listener attaches, and `AppWindow`'s
`will-frame-navigate` handler blocks subframe navigation away from `about:blank`.

Inferred from code; not run. The message iframe allows forms
(`features/message-list/lib/email-frame.tsx:222`, `sandbox="allow-forms
allow-same-origin"`), the sanitizer keeps `form`, `button`, `action` and `method`
(`src/services/sanitize-transformer.ts:3-6` and `AllowedAttributes`), `EventedIFrame` has no
submit handler, and `will-navigate` guards only the top frame
(`src/main/app-window.ts:363-369`).

- Cost: a submit button styled as a link loads the attacker's page inside Tern's chrome ("re-enter
  your password to view this message") and receives what the user types. That page also loads
  remote images whatever the setting.
- Fix: intercept `submit` in `EventedIFrame` and open the action externally after a confirm, or
  drop `allow-forms`. Add a `will-frame-navigate` handler in `AppWindow` that blocks
  subframe navigation away from `about:blank`.
- Size: ~40 TS.

### TD11. App CSP duplicated and weak; signature preview unsanitised

Done (roadmap/security, f3bd26b43). `AppContentSecurityPolicy` is sent by `main.js` and copied in
`static/index.html`; a spec fails if they differ. `script-src 'self'`, `default-src 'self'
mailspring:`, `frame-src 'none'`, `object-src 'none'` (it was unquoted), and `connect-src` lists
the Google/Microsoft OAuth endpoints and api.github.com. Autoconfig, one-click unsubscribe and
Save Image fetch hosts from user data, so they go through `ExternalFetch` (main-process
`net.fetch`, no cookies, Tern windows only). The signature preview runs through
`SanitizeTransformer`. The `app://` scheme move is still open. Review fixes (a7e9107e6):
`static/db-migration.html` and `db-vacuum.html` carry the same policy and the drift spec checks
every static page. In a file: window `script-src 'self'` still matches any local file, which the
`app://` move would fix.

Verified. The header policy (`src/main/main.js:456-470`) and the meta tag
(`static/index.html:6`) are meant to match but the header adds `'unsafe-inline'`, so only the meta
tag blocks inline handlers. `default-src *` leaves `connect-src` and `frame-src` open. The app
loads from `file://`, so `script-src 'self'` matches any local file, including attachments under
`<config>/files`. The signature preview injects raw `signature.body`
(`features/composer-signature/lib/preferences-signatures.tsx:169`).

- Cost: drop the meta tag, or open a window without it, and inline handlers run in a
  Node-enabled renderer. Once settings live in the engine (stage 3) and web or MCP can write
  signatures, a signature is HTML injected from another client.
- Fix: one CSP constant for header and meta, no `'unsafe-inline'`, `connect-src` pinned to the
  hosts the renderer fetches (OAuth token and userinfo endpoints, the backend). Run the signature
  preview through `SanitizeTransformer`. Later, load the app from an `app://` scheme so `'self'`
  stops meaning the whole disk (~200 lines plus spec fixes).
- Size: ~40 TS. The sanitising should land before stage 3.

### TD12. Mail in fixed `/tmp` names; engine files at default umask

Engine half done (roadmap/client-next, stage 11, 5b1974bb3). The engine sets its umask to 077
at start (`platform::restrict_umask`), so the database, logs, attachments, `accounts.json` and
`secrets.enc` are created 0600 in 0700 directories, and `serve` makes its config directory 0700
(`platform::uds::restrict_dir`, which also drops group and other bits from an existing one).

Desktop half done (launch security pass, f087ddab5). `src/utils/private-temp.ts` writes each
file 0600 into its own `fs.mkdtemp` directory (0700) and removes the directories a window made
when it unloads. The clipped message, raw source (the engine writes it into such a directory),
account sync logs (now HTML-escaped), the onboarding log and dragged contacts use it; .eml
staging and pasted files use `mkdtemp` and keep their own lifetimes. The print window gets the
page over `app-api:print:open`; main writes `print.html` and `print.messages.js` into a
`tern-print-` mkdtemp directory and removes it when the print window closes, so no window
passes a path and nothing loads a script from a shared name.

Verified. On Linux `app.getPath('temp')` is `/tmp`. These are written with `writeFileSync`
defaults (0644 under the usual umask) at predictable names and never removed:
`print.html` and `print.messages.js`, which the print window loads as a `<script>`
(`features/print/lib/print-window.ts:19-20`, `:73`, `:79`, `:125-126`); `<message.id>.html`
(`features/message-list/lib/message-item-body.tsx:112-115`); raw RFC822 at
`<temp>/<message.id>` (`message-controls.tsx:159-162`); sync logs
(`preferences-account-details.tsx:205-207`, `onboarding/lib/form-error-message.tsx:23`).

The engine creates `tern.db`, logs and attachments at the default umask
(`crates/tern-sync/src/mail_processor.rs:712-721`, `crates/tern-core/src/log.rs:146`,
`crates/tern-store/src/schema.rs:114`); only `run/` is private (`apps/engine/src/platform.rs:153-166`).
Today the desktop chmods the config dir to 0700 (`src/main/config-persistence-manager.ts:37-45`),
which covers those files, so the engine half matters when the engine creates its own directory
(stage 11 `--detached`, `--config-dir-path`, hosted).

- Cost: on multi-user Linux, other users can read message bodies, raw source and logs from
  `/tmp`. A pre-created `/tmp/print.messages.js` is a script-injection race into the print window,
  depending on `protected_regular`. Leftover copies stay on disk on every platform.
- Fix: per-use `fs.mkdtempSync` (0700), files at 0600, deleted when the window closes; pass print
  data over IPC instead of a script file. Engine: `create_private_dir` for config, files and
  logs, open logs and attachments 0600, or `umask 077` at start.
- Size: ~80 TS now; ~40 Rust before stage 11.

### TD13. Outdated vendored parsers run on attachments

Done (launch security pass, 876394b87). Quick preview uses SheetJS 0.20.3 (past both CVEs below,
from the SheetJS CDN tarball), Prism 1.30.0 (rebuilt from the npm package with the 1.15 bundle's
languages and okaidia theme) and DOMPurify 3.4.16, the dependency's own file.
`tests/mail/quick-preview-vendor.test.ts` fails when the vendored DOMPurify differs from
`node_modules` or SheetJS is below 0.20.2, and checks that every vendor path `renderer.html`
loads exists. Both preview windows set `sandbox: true`. Not done: pdf.js stays at 4.3.136 (past
CVE-2024-4367, fixed in 4.2.67), and the libraries are still copied in, so npm audit doesn't see
SheetJS, Prism or pdf.js.

Engine side (checked in the pre-launch engine pass): nothing to do. The engine has no copied-in third-party
parsers: MIME, ICS and vCard parsing use crates.io crates (`mail-parser`, `html5ever`,
`calcard`) that `cargo deny check` (CI, `engine.yml`) checks against RustSec advisories, and the
iCalendarLib port in `tern-dav/src/ical.rs` is our own code. Engine-side parser hardening is
TD45.

Verified. Quick preview uses copied-in `vendor/xlsx-0.18.5`, `prism-1.15.0` (2018),
`pdfjs-4.3.136` and a second copy of DOMPurify (`src/quickpreview/index.ts:63-99`, thumbnails in a
hidden window at `:333`). No package manager or audit tool sees them. SheetJS 0.18.5 predates
CVE-2023-30533 (prototype pollution, fixed 0.19.3) and CVE-2024-22363 (ReDoS, fixed 0.20.2).

- Cost: opening or thumbnailing a crafted attachment hits known bugs. The preview window is
  context-isolated with a CSP, so the damage is a hung or misleading preview (inferred). Nobody is
  told when an update is due.
- Fix: load them from package deps (xlsx from the SheetJS CDN tarball, since npm's copy is stale;
  `prismjs`; the existing `dompurify`) and copy them at build time, or drop xlsx thumbnails. Set
  `sandbox: true` explicitly on preview windows.
- Size: ~1 day.

### TD14. CI: no dependency updates, no npm licence gate, mutable action tags with signing secrets

Verified. No `.github/dependabot.yml`. `engine.yml:37` and `backend.yml:40` run cargo-deny, but
the desktop has no equivalent, and `build/generate-licenses.js` collects licences without checking
them, so the GPL-compatibility rule is enforced for Rust only. Release jobs holding Azure, Apple,
AWS and Snap secrets use mutable tags (`azure/trusted-signing-action@v0` at
`desktop-build-windows.yml:85`, `:148`; `snapcore/action-publish@v1` at `desktop-build-linux.yml:157`;
`aws-actions/configure-aws-credentials@v6`), while `engine-release.yml` pins SHAs. Desktop builds
run only on `workflow_dispatch`, specs only on Linux, `playwright/tests` in no workflow, and four
debug PNGs are committed under `playwright/test-results`.

- Cost: packaging and signing regressions show up at release time. A licence-incompatible npm
  dependency can ship. A compromised tag gets the signing keys.
- Fix: dependabot for npm (both dirs), cargo and github-actions. A licence allowlist step in
  `desktop-test`. Pin third-party actions to SHAs. An unsigned Linux `build.js` run on PRs that
  touch package files or `build/`, and a weekly macOS/Windows spec run. Delete the PNGs and
  gitignore the directory.
- Size: 1–2 days.

Done except the PR builds and weekly specs. `.github/dependabot.yml` covers cargo, npm
(`apps/desktop`, `apps/web`) and github-actions, weekly and grouped. Every action is pinned to a
commit SHA with its version in a comment. `.github/scripts/check-npm-licenses.mjs` checks the
shipped npm packages against deny.toml's allowlist in `desktop-test`, the new `web` workflow
and `make lint`. The playwright PNGs are gone. Left: the unsigned Linux `build.js` run on PRs
and a weekly macOS/Windows spec run.

### TD15. Huge emails parsed in full twice per render despite the clip

Verified. `MessageBodyProcessor` clips at 300,000 chars (`src/flux/stores/message-body-processor.ts:162`),
but `ConditionalQuotedTextControl` passes the raw `message.body` to `hasQuotedHTML`
(`features/message-list/lib/message-item-body.tsx:206-209`), which builds a full
`DOMParser` document (`src/services/quoted-html-transformer.ts:31-36`). `EmailFrame._writeContent`
runs `removeQuotedHTML` (parse plus serialize) on every write, including every theme change
(`email-frame.tsx:41`, `:86-89`). Neither is cached.

- Cost: a multi-MB HTML mail is parsed on the UI thread at least twice per render, defeating the
  clip. Switching themes redoes it for every open message.
- Fix: compute `hasQuotedText` and the stripped body once, from the clipped body, in
  `MessageBodyProcessor`, and cache them with the processed result.
- Size: ~100 TS.

### TD16. Localisation: placeholder off-by-one, drifting keys, no plurals

Verified.
1. `localizedReactFragment` maps `%1$@` to `subs[1]` (`src/shared/intl.ts:253`) while `localized` maps
   it to `subs[0]` (`:219-233`). zh-CN "Sign in with %@ in %@ your browser." uses `%2$@…%1$@`, so
   onboarding renders without the provider name (`onboarding/lib/oauth-signin-page.tsx:206-209`).
2. The extractor collapses whitespace in keys (`apps/desktop/scripts/format-localizations.js:46`),
   the runtime looks up the raw string, so multi-line template literals never match
   (`plugins/activity/lib/dashboard/root.tsx:686-689`).
3. `lang/en.json` has 902 keys while ~514 source strings are missing from it; de.json lacks ~43%.
   The extractor hasn't run since the Tern additions, and rebrand edits change English keys and
   drop their translations.
4. No plurals; counts are concatenated (`activity-list-item-container.tsx:45-51`).
5. moment's locale comes from `navigator.language` (`src/utils/date-utils.ts:4`), UI strings from
   `core.intl.language`.

- Cost: broken strings in zh-CN; close to half the UI untranslated in de/fr/ja/es unnoticed.
  Stage 4 and stage 13 push localised text into the engine and outgoing mail.
- Fix: `subs[Number(match[1]) - 1]` with `(\d+)`. Same whitespace normalisation in `localized()`
  and the extractor. Run the extractor in CI and fail on diff. `localizedPlural(count, {one,
  other})` on `Intl.PluralRules`. `moment.locale(getCurrentLocale())` after
  `initializeLocalization`. Keep English-as-key.
- Size: <30 lines for the bugs; S–M for CI and plurals. Before stage 4 at the latest.

### TD17. Nine help links point at a 15-line README

Verified. The rebrand pointed every upstream help link at `github.com/ternmail/ternmail#readme`:
Office 365 and Yahoo IMAP and app passwords (`onboarding/lib/account-providers.tsx:19`, `:59`;
`create-page-for-form.tsx:267`, `:270`), search syntax (`thread-search/lib/search-bar-util.ts:19`),
templates (`composer-templates/lib/preferences-templates.tsx:115`), workspace and default client
(`preferences/lib/tabs/workspace-section.tsx:63`), activity (`activity/lib/dashboard/root.tsx:597`),
and Help > Help (`src/main/application.ts:473`).

- Cost: users stuck at the commonest failures (app passwords, tenant-disabled O365 IMAP) get
  nothing, and file issues instead.
- Fix: short docs pages (app passwords per provider, O365 IMAP/SMTP AUTH, templates, default
  client, search grammar) behind one `HelpURLs` module. The search page documents the grammar
  stage 9 ships.
- Size: ~1 day docs plus ~30 TS.

### TD18. No update check at all

Verified. `UPDATE_FEED_BASE_URL` is `''` (`src/main/autoupdate-manager.ts:20-23`, `:49-52`), the
manager stays `unsupported`, update menu items are hidden, and `docs/backend-plan.md:49` defers a
feed.

- Cost: installs stay on their Electron/Chromium (44.5.1) and miss security and sync fixes with
  no signal to update.
- Fix: until a feed exists, an opt-out check of the GitHub releases API on launch and daily,
  reusing `update-notification.tsx` to link to the release. No automatic install.
- Size: ~80 TS. Before the first public release.

### TD19. Accessibility: dialogs, focus, announcements, contrast, motion

Verified. No component has `role="dialog"` or `aria-modal`. `FixedPopover` moves focus only on
`animationend` and never restores it (`src/components/fixed-popover.tsx:109-111`, `:170-186`);
`Modal` focuses a field but doesn't trap or restore (`src/components/modal.tsx:28-55`). No
`aria-live`, `role=status` or `role=alert` anywhere, so undo toasts, the offline bar and send
errors aren't announced. About 25 `<div className="btn" onClick>` controls have no role or
tabIndex (Reset Cache, Reconnect, Try Again, onboarding Back:
`preferences-general.tsx:118`, `:121`; `preferences-account-details.tsx:385-406`;
`oauth-signin-page.tsx:234`; `create-page-for-form.tsx:333`). No pane-cycling shortcut.
`--muted-foreground` (`@text-color-very-subtle`; `static/styles/tokens/defaults.css:27`) is
~3.2:1 on white, used in 129 places. No `prefers-reduced-motion` rule outside vendored pdf.js.

- Cost: keyboard and screen-reader users can't operate snooze, move/label pickers, account
  recovery or onboarding; focus lands on `<body>` after every popover. Collapsed snippets and
  timestamps fail WCAG AA.
- Fix: a shared dialog wrapper for `FixedPopover` and `Modal` (role, `aria-modal`,
  `aria-labelledby`, focus trap, restore to opener, focus on mount). A polite live region in
  notifications and undo-redo. `<button>` for `div.btn`. A pane-cycling command. Raise very-subtle
  to ~35–40% fade and check dark themes. A global reduced-motion block, after the focus fix
  (disabling animations today would break popover focus).
- Size: ~3 days. Land it before stage 5 freezes plugin API 1.27 if `FixedPopover` props change.

### TD20. Body-fetch placeholder rows are never retried

Verified. `sync_message_bodies` reserves up to 30 messages with `MessageBody(id, NULL)` and no
`fetchedAt` (`crates/tern-sync/src/worker.rs:661-667`) and fetches them one at a time. A connection
error returns early (`:719-720`), leaving the rest as NULL rows. Candidate selection skips ids
with a row and the purge needs `fetchedAt < now-14d` (`:559`), so they're never background-fetched
again. Inherited (`SyncWorker.cpp:1588-1608`).

- Cost: each network blip during body sync leaves up to 29 messages without offline bodies or
  body search text. Opening them still fetches on demand.
- Fix: on early return, delete placeholders not attempted. Mark real failures with a time so a
  retry window clears them.
- Size: ~40 Rust plus one scenario. Before stage 9.

### TD21. Threading ignores In-Reply-To and never merges

Verified. Non-Gmail lookup uses only the message's Message-ID and References
(`crates/tern-store/src/search.rs:131-157`, `crates/tern-sync/src/mail_processor.rs:205-217`), though
In-Reply-To is parsed (`crates/tern-store/src/models/message.rs:139`). When references hit two
threads, `LIMIT 1` with no `ORDER BY` picks one and they're never merged. Inherited
(`MailProcessor.cpp:162-227`).

- Cost: replies from ticketing systems, form mailers and some mobile and webmail clients start new
  threads; grouping depends on arrival order.
- Fix: add In-Reply-To ids to the lookup and to `upsert_thread_references` (~50 lines, now). Later,
  pick the oldest match deterministically and merge the rest via `Thread::recount_from_messages`
  (~300 lines plus scenarios).

### TD22. Desktop specs need a release engine build

Inferred; the fix is untested. Spec mode boots the full app, which runs `engine.migrate()`
(`src/main/application.ts:105-107`) before any spec loads, so `desktop-test.yml:66` builds the
engine in release first. Without an engine the run exits 0 having run nothing, which is why CI
greps for "N passing". The runner is a 2668-line `@ts-nocheck` fork of Jasmine 1.3
(`spec/spec-runner/jasmine.js`) with async wrappers (`spec-runner.ts:44`). The root has
`@types/jasmine` 1.3 and an unused jasmine 2.x.

- Cost: a desktop-only PR waits for a cargo release build; local runs need a built engine. New
  contributors learn Jasmine 1.3 idioms (`andReturn`, `waitsForPromise`).
- Fix: skip migrate in spec mode (specs already stub `DatabaseStore._query`,
  `master-before-each.ts:64`), then drop cargo from `desktop-test`. Use `node:test` or vitest for
  new DOM-free modules (engine client, supervisor, settings mirror) from stage 6 on. Leave the
  213 legacy specs where they are.
- Size: small.

### TD23. No permission handlers; `--harmony`

Done (roadmap/security, 560387678). `main.js` installs `src/main/permission-policy.ts` on every
session as it is created (`session-created`, plus the default session at `ready`), which covers
default, `quickpreview`, `in-memory-only` and any plugin partition. Request and check handlers
allow only `notifications` and `clipboard-sanitized-write` (CopyButton, thread sharing), only to
the main frame of an `AppWindow` on a `file:` URL; dev mode logs each denial once.
Tern's own notifications come from main and need no permission, so `notifications` is there for
plugins. `--js-flags=--harmony` is gone (no staged-feature APIs in the tree, TS targets es2024).
The autoplay switch stays: new-mail sounds play through an `Audio` element with no user gesture.
Review fix (a5d05e112): the main frame of the bundled pdf.js viewer in quick preview also gets
`fullscreen` (Presentation Mode) and `clipboard-sanitized-write` (select-all copy).

Verified. No `setPermissionRequestHandler` or `setPermissionCheckHandler` on the default,
`quickpreview` or `in-memory-only` sessions, so Electron grants media, geolocation, clipboard-read
and display capture by default. `src/main/main.js:369-370` passes `--js-flags=--harmony`
(staged V8 features in Node-enabled renderers) and an autoplay-without-gesture switch.

- Fix: deny-by-default handler on every session, allowing notifications only for
  `AppWindow` contents. Drop `--harmony`. Keep autoplay only if notification sounds need it.
- Size: ~30 lines.

### TD24. Abandoned single-use dependencies

Verified. `optimist` 0.4.0 with an argument-swallowing workaround (`src/main/main.js:120`,
`:139-144`). `getmac` shells out at startup in every process for an empty Sentry DSN
(`src/error-logger-extensions/sentry-error-reporter.js:2`, `:177`). (`tld` is gone: phishing-detection
shows the engine's `warnings`, which read the current public-suffix list.) Reflux 0.1.13 is used
only for `createAction` with `sync = true` set by hand on every action (`src/flux/actions.ts:79-85`).
Root `package.json` carries unused `chalk`, `temp`, `targz`, `license-extractor`, `devtron`,
`electron-playwright-helpers`, jasmine 2.x and `node-gyp`, and deprecated `glob` 7, `rimraf` 2,
`fs-extra` 2, `fs-plus` 2.

- Fix: `util.parseArgs`; a random per-install id (see TD56); a ~50-line `createAction`
  keeping Reflux's `listen`/`trigger`/`sync`/`actionName` surface; drop the unused root deps and use
  `node:fs` (`cpSync`, `rmSync`, `globSync`) in `build.js` and postinstall. rx-lite and the
  `emorikawa/debug` fork wait for TD38 and TD53.
- Size: 1–2 days.

### TD25. First-run tutorial and settings advertise features that can't work

Verified. Two of three first-run slides promise "LinkedIn profiles, Twitter bios" and open/click
tracking (`features/onboarding/lib/page-tutorial.tsx:5-25`), both inert without a backend.
General > Notifications shows repeated opens/clicks (`src/window/config-schema.ts:359-363`). The config
load-error dialog says "continue using N1" (`src/main/config-persistence-manager.ts:64`). The
shortcut presets offer "Inbox by Gmail" (`keymaps/templates/Inbox by Gmail.json`), shut down in
2019.

- Fix: slides for snooze, send later, templates and rules. Hide the tracking notification setting
  while capabilities report tracking off. Fix the N1 string; rename or drop the preset. Disabled
  toolbar toggles are already tracked in `docs/pro-features-backend.md`.
- Size: 0.5 day.

## Fold into a stage

Ordered by the stage they belong to.

### TD26. Engine protocol types hand-copied in desktop and web (before stage 1)

Done (thin-clients stage 3). tern-ipc derives `schemars::JsonSchema` on its method, notification,
model (`models.rs`) and task (`tasks.rs`) types; `cargo run -p tern-ipc --bin tern-ipc-schema`
writes `packages/client/schema/tern-ipc.json` and the `query` allowlist (now from
`query_attrs.rs`), and `packages/client/scripts/generate.mjs` turns the schema into
`@tern/client`'s TS types and constants. The desktop's `src/engine/protocol.ts` and the web's
`src/engine/rpc.ts` and `types.ts` re-export it, keeping only the JSON-RPC envelope, socket
helpers and a few narrowed types. `crates/tern-sync/tests/view_structs.rs` checks what sync
writes, `query` returns and deltas carry against the schema; tern-ipc's tests and the
`protocol` CI job fail on a stale schema, allowlist or TS. Left: the `integrations.*` results
(other than `integrations.test`) and `ContactDetails.data` are untyped.

Verified. `src/shared/engine-protocol.ts:4-8` says its types are hand-written from `crates/tern-ipc` and
generation waits until "the surface grows". `apps/web/src/engine/rpc.ts` (279 lines) has its own
envelope, `PROTOCOL_VERSION` and method map, and `apps/web/src/engine/types.ts` (421 lines) copies
the model JSON. The protocol design deferred generation to the desktop client refactor, which scheduled no generation step while adding ~30 methods and
notifications. No `schemars`, `ts-rs` or `specta` in any `Cargo.toml`; `packages/` holds only a
README.

- Cost: every new method is three hand edits; drift shows up as runtime `undefined`s, and stage 8
  routes every read through these types. tern-client (T20) shares types on the Rust side only.
- Fix: derive TS (ts-rs or specta) or JSON Schema (schemars plus json-schema-to-typescript) from the
  tern-ipc request, response and notification types and the tern-store model structs into
  `packages/engine-protocol`; desktop and web import it; CI fails when regeneration gives a diff.
  Keep only runtime helpers (socket path, framing) in `engine-protocol.ts`. The desktop's
  `flux/models` classes stay, since they carry attribute metadata.
- Size: ~300–500 lines of derives and a generator script.

### TD27. TypeScript non-strict, lint stack EOL (before stage 1)

Verified. `tsconfig.json` sets no `strict`, `noImplicitAny` or `strictNullChecks`; ~700 explicit
`any`. `.eslintrc` turns off `no-explicit-any`, `no-unused-vars` and `no-var-requires`. ESLint 8 is
EOL and `@typescript-eslint` 5.62 supports TS below 5.2 against TS 5.7.3
(`apps/desktop/package.json:45-58`). The react, import and jsx-a11y plugins load without their rule
sets, there's no react-hooks rule, specs and `.js` files (main.js, error reporter, build scripts)
aren't linted, and the `lint` script runs `--fix` (`:76`). `settings["import/core-modules"]` lists
`mailspring-exports` and friends, so `import/no-cycle` can't see TD36's cycles. `apps/web` already
uses a flat config.

- Cost: the riskiest new code (supervisor, reconnect and timeout contract, query transport,
  `Engine` export) gets 2017-era checking; unawaited promises, the typical bug when calls become
  async RPC, go unflagged even though type info is configured.
- Fix: `tsconfig.strict.json` with `strict: true` and an explicit include list (`src/engine-*.ts`,
  `src/main/engine-supervisor.ts`, the stage 8 transport, the `Engine` export), run as a second
  `tsc --noEmit` in CI; each stage adds what it rewrites. ESLint 9 flat config with typescript-eslint
  8; `react-hooks`, `no-floating-promises`, `no-misused-promises`, `react/no-string-refs`,
  `react/no-find-dom-node`, warn on legacy files and error on strict-listed ones; check-only `lint`.
- Size: 2–3 days to start; grows per stage.

### TD28. No Electron fuses (after stage 2 and the database-agent swap)

Done (launch security pass, bc5f4697b). `build/build.js` flips the fuses with `@electron/fuses`
in `afterExtract`, before the app is copied in and signed: RunAsNode,
`EnableNodeOptionsEnvironmentVariable` and `EnableNodeCliInspectArguments` off;
`EnableCookieEncryption` and `OnlyLoadAppFromAsar` on; `EnableEmbeddedAsarIntegrityValidation`
on for macOS and Windows, where the packager embeds the asar header hash (`ElectronAsarIntegrity`
in Info.plist, a resource in the exe), and off on Linux, where Electron doesn't check it.
Unsigned arm64 builds get their ad-hoc signature reset. Left at Electron's defaults:
`GrantFileProtocolExtraPrivileges` (on, since the windows run `file://` pages until the app
scheme in TD11), `LoadBrowserProcessSpecificV8Snapshot` and `WasmTrapHandlers`. Both
prerequisites are gone: the MCP bridge was deleted (Preferences only replaces the Claude Desktop
entry it wrote), and the database agent is a utility process (`src/main/ipc/database-agent.ts`):
main forks `agent.js` for each window that asks and hands the window a MessagePort, and queries
the agent fails on or hasn't answered when it exits run in the window. Nothing else runs the app
with ELECTRON_RUN_AS_NODE (`scripts/check-native-abi.js` runs the dev Electron). Unverified until
the QA build: a signed and notarised build with the fuses, the asar integrity check, and the
utility process loading `agent.js` from inside app.asar.

Verified. Nothing in the repo uses `@electron/fuses`; `build/build.js` has `osxSign` (~:315) and an
`afterCopy` list (`:370-378`) with no fuse step. RunAsNode, `NODE_OPTIONS`, `--inspect` and loading
outside the asar are allowed and asar integrity isn't checked. Two things need RunAsNode:
`internal_packages/mcp-server/lib/claude-desktop-config.ts:63` (its comment at `:13-15` says so) and
`childProcess.fork(AGENT_PATH)` in `src/flux/stores/database-store.ts:357`. Its in-process fallback
(`:381-392`) only triggers when `fork()` throws, and with RunAsNode off it would more likely launch
a second app instance (inferred).

- Cost: any local process can run signed Tern as a Node interpreter and borrow its keychain ACL
  (the safeStorage key over account secrets until stage 11), TCC grants and the apple-events
  entitlement without a prompt (inferred, not tested). A patched `app.asar` keeps a valid
  signature.
- Fix: in `afterCopy`, before signing: RunAsNode, `EnableNodeOptionsEnvironmentVariable` and
  `EnableNodeCliInspectArguments` off; `EnableEmbeddedAsarIntegrityValidation`,
  `OnlyLoadAppFromAsar` and `EnableCookieEncryption` on. Prerequisites: stage 2 deletes the MCP
  bridge, and the database agent moves to `utilityProcess.fork` (or waits for SQLite removal after
  stage 10). After stage 11 secrets sit under the engine's own keychain item, so fuses still matter
  for TCC, entitlements and asar tampering.
- Size: ~50 lines plus re-checking signing and notarisation. Before any public signed release.

### TD29. No plugin trust model before the `Engine` export and service plugins (before stage 2)

Inferred from the code. Plugins load from `<config>/packages` with full Node
(`src/plugin-host/package-manager.ts:35-40`) after a warning dialog (`:167-178`). `Engine.request` (stage 5)
goes over the desktop's own local connection, so T13's local-only class never applies to plugin
calls: a plugin can call `account.setSecrets`, `engine.logs.tail` or `engine.resetDatabase`. Stage 19
lets a manifest install a launchd or systemd unit. Packages record no source or hash.

- Cost: a compromised plugin can read every secret and gain login persistence, and the refactor
  didn't name the risk. Changing the manifest or `Engine` after 1.27 is a breaking change.
- Fix: document that plugins are fully trusted code. Tag `Engine.request` with the calling package
  (the `deprecate()` stack walk can find it) and refuse local-only methods unless the manifest
  declares them and the user approved at install. Show the exact binary and unit before
  `tern service install`. Record install source and hash in Preferences > Plugins (TD30).
- Size: ~200 TS plus docs. Before stage 2 sets `PLUGIN_API_VERSION` and the `tern` block.

Done in part (roadmap/client-next, stage 5). `src/engine/engine-api.ts` and
`docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md` (Trust Model) say plugins are fully trusted code and
that the check below is no security boundary. `Engine.request` finds the calling package from the
call's stack (the first frame outside the app's `src/` and `node_modules`, matched to a loaded
package's directory or its realpath; a frame outside the app in no package is an unknown caller,
refused) and refuses a T13 local-only method (`account.*` writes, `secrets.*`, `credentials.*`,
`server.*`, `engine.logs.tail`, `engine.setLogLevel`, `engine.resetDatabase`,
`engine.shutdown`) unless the package's `tern.engineMethods` lists it and the user allowed it.
The user is asked on the first call, not at install (no install step reads the manifest yet); a yes
is kept in `core.plugins.engineMethodApprovals`, a no for the session. The app's own code and
bundled packages are never asked. Stage 19 (roadmap/client-next) did the rest: Preferences >
Plugins runs `tern service install --dry-run` and shows the binary, its SHA-256, the copy that
will run and the exact unit text before anything is written, and the install refuses a binary
whose hash changed since (`--expect-sha256`) or any change to what it writes, args and engine
unit included (`--expect-plan` with the dry run's `planSha256`); the unit runs a copy in the profile, so a later
plugin update changes nothing until the next install. `<config>/services.json` records each
service's source, binary, hash and unit, and `core.plugins.installs` each plugin installed
through Tern with its source directory and a SHA-256 over its files, shown in the pane. Still
open: plugins copied into `packages/` by hand have no record, and approvals for
`tern.engineMethods` can't be seen or revoked in the pane (TD30).

### TD30. Plugins can't be disabled from the UI; dead and placeholder plugins (stage 2)

Verified. Preferences has no Plugins tab (`features/preferences/lib/main.tsx:14-55`).
`core.disabledPackages` is honoured only for `isOptional` packages (`src/plugin-host/package-manager.ts:107-108`),
so a third-party plugin can't be disabled short of deleting it, and optional built-ins only by
editing `config.json`. `phishing-detection` ships in the template's `disabledPackages`
(`default-profile/config.json`) and is marked `isHiddenOnPluginsPage`, which no code reads. The
template also lists five packages that no longer exist (nylas-private-salesforce, keybase,
composer-markdown, composer-scheduler, composer-mail-merge). `thread-sharing` renders a permanently
disabled toolbar button (`thread-sharing/lib/thread-sharing-button.tsx`).
`internal_packages_disabled/thread-unsubscribe` overlaps `list-unsubscribe` and nothing loads it.
`github-contact-card` (off by default) puts the contact's email unencoded into a GitHub search URL
(`lib/github-user-store.ts:64`).

- Cost: a crashing plugin can't be switched off in-app; a cheap anti-phishing warning is
  unreachable; dead code inflates `docs/plugins-inventory.md` counts.
- Fix: a Preferences > Plugins pane with toggles and load errors, honouring `disabledPackages` for
  every non-core package. Enable phishing-detection by default or fold it into message-list. Delete
  `internal_packages_disabled` and the stale template entries. Hide thread-sharing until a
  capability exists. `encodeURIComponent` in github-contact-card and label it as sending data to
  GitHub, or delete it.
- Size: ~250 TS. With stage 2's semver check.

### TD31. `webviewTag` on every window for a component nothing renders (stage 2)

Done (launch security pass, be6c6155b). App windows set `webviewTag: false`, and the
`will-attach-webview` guard stays as a backstop. `Webview` in `mailspring-component-kit` is
deprecated in plugin API 1.35.0 (a package that reads it is warned once) and removed in 2.0.0;
it renders an inert tag, and `chromium-net-errors` goes with it. The `tern://plugins` route and
the onboarding back button's webview check are gone. The asset protocol searches
`<config>/dev/packages` in dev mode only, and the profile's `packages` not in safe mode, as
PackageManager does.

Verified. The only `<Webview>` user was the Mailspring ID sign-in (removed in 27f713bf6), but
`src/main/app-window.ts:128` still enables the tag in every Node-enabled window, the
`will-attach-webview` comment (`:163-167`) names an onboarding guest that's gone, and the component
is exported to plugins (`src/plugin-api/mailspring-component-kit.js:70`). `webview.tsx:152-161` opens
any http(s) URL a guest logs to the console, with no user gesture. Also dead:
`onboarding/lib/page-top-bar.tsx:12-14`, `chromium-net-errors` (only used by `webview.tsx`), the
`changePluginStateFromUrl` route with no listener (`src/main/application.ts:1061-1062`), and the
protocol handler serving `dev/packages` outside dev mode (`src/main/asset-protocol-handler.ts:20-22`,
unlike `package-manager.ts:36-39`).

- Fix: deprecate the `Webview` export through `deprecate()` in stage 2 and remove at 2.0.0. Set
  `webviewTag: false`, remove the dead route and back-button code, search `dev/packages` only in
  dev mode. Keep the attach guard as a backstop.
- Size: ~100–200 lines, mostly removals.

### TD32. Reset / Rebuild Cache deletes local-only plugin metadata (before stage 4)

Verified, except that thread ids change after resync, which is inferred. `reset_account`
(`crates/tern-store/src/migrate.rs:101-108`) runs `DELETE FROM ModelPluginMetadata` and
`DetatchedPluginMetadata` (`crates/tern-store/src/schema.rs:106-107`) and sets the metadata cursor to
0 "so metadata is fetched again on resync". Under D3 LocalOnly is the default and nothing is
fetched. The whole-database delete after a database error (`src/main/application.ts:349-374`)
and per-account Reset in Preferences (`preferences-account-details.tsx:168` →
`src/engine/client/bridge.ts` → `--mode reset`) both hit this today. Thread ids are `t:<first message
inserted>` (`crates/tern-core/src/ids.rs:126-128`), so reattaching by thread id wouldn't work anyway.
With `HttpMetadataBackend` configured, the cursor reset does replay from the backend.

- Cost: reset drops scheduled sends, reminders and snoozes without saying so; snoozed threads stay
  in Snoozed for good. Stage 10's `engine.resetDatabase` does the same for every
  account.
- Fix: before deleting, stash each model's metadata into `DetatchedPluginMetadata` under the
  headerMessageId keys legacy import uses (`legacy_import.rs:660-711`), stop deleting that table in
  reset, and let `attach_pending_thread_metadata` reattach on resync. For whole-database deletes,
  export best-effort (or move the file aside) first, since a corrupt database may not read. A
  harness scenario: snooze, reset, resync, unsnooze fires.
- Size: ~300–400 Rust plus scenario. Before stage 4 makes the engine the only owner of
  snooze and send-later state.
- Status (2026-10-18, `roadmap/client-next`): fixed for `--mode reset`, `account.reset` and
  `engine.resetDatabase` (`crates/tern-store/src/metadata_stash.rs`; scenarios
  `reset-keeps-plugin-metadata`, `expiration-reset-keeps-snooze`). Removing an account does not
  keep it: `account.reset {keepMetadata: false}` and `engine.resetDatabase {keepMetadata: false}`
  (from the desktop's account removal) delete it, since a re-added account has the same id
  (`remove-account-drops-plugin-metadata`). Still open: the desktop's
  file-deleting fallbacks (Rebuild after the engine stopped, the failed-`migrate` rebuild at
  launch) delete the database without the stash.

### TD33. No outbox; failed silent sends pop a composer (silent fix now; queue in stages 4 and 13)

Verified. A send runs once (`crates/tern-tasks/src/send.rs:9-12`); a connection error ends the task
with an error dialog or popout composer (`src/flux/stores/draft-store.ts:667-690`). Nothing in the
send path reads `OnlineStatusStore`. `SendDraftTask` checks `silent` at `:115` and `:142` but not
in `onError` (~`:150-201`), so a snooze or reminder resurfacing message that fails offline opens a
composer for a draft the user never wrote.

- Cost: sending on flaky Wi-Fi fails loudly and needs a manual retry. Stage 4's executor and stage
  13's `draft.send` would inherit fire-once, so a scheduled send due while offline fails.
- Fix: now, `onError` only logs for silent tasks (~10 TS). In stages 4 and 13: on
  `SyncError.offline`/`retryable`, keep the send queued until the account reconnects and show
  "Queued, sends when online" on the draft (~400 Rust, ~100 TS). Shares the retry design with TD43.
- Status (2026-10-18, stage 4): `onError` logs and returns for silent tasks. Sends the engine
  starts itself (`retryWhenOnline`: scheduled sends, undo send, resurfacing messages) go back to
  `remote` on a retryable SMTP error before the server accepted them, for up to a day after the
  first one. The foreground skips such a send for 2 minutes (or until `wake-workers`) and goes on
  fetching and idling meanwhile, so an account whose SMTP server alone is down stays online
  (`expiration-send-offline`). Open
  for stage 13: the same for sends from the composer (`draft.send`), and the "Queued, sends when
  online" state on the draft.
- Status (2026-10-18, stage 13, roadmap/client-next): `draft.send` queues its `SendDraftTask` with
  `retryWhenOnline`, so a composer send through the engine stays queued on an offline or
  retryable failure like the executor's. The desktop reads the engine's `_offlineSince` on the
  task (`SendDraftTask.offlineSince`) and the draft list shows "Queued, sends when online" while
  such a send is pending. Done for engines with `draft-pipeline`; a send through a plugin's send
  action still runs once.

### TD34. Message bodies render inside the Node-enabled renderer (before stage 5)

Done (roadmap/client-next). The reading pane's message bodies render in the message frame
(`src/message-frame`): an iframe on `tern-message://frame/message.html`, a standard secure scheme
the main process serves (`main/message-frame-protocol-handler.ts`), sandboxed with
`allow-scripts allow-forms` and no `allow-same-origin`, so it is cross-origin to the app window and
Chromium runs it in its own sandboxed renderer process without Node (checked from the spec with
`getAppMetrics`). Its CSP (TD2's message policy plus `script-src` for the frame's one script and
`base-uri 'none'`) comes as a header and as the first element; the remote-images variant is a
second page URL, and `will-frame-navigate` only lets the app window, never the frame itself, load
either. The frame script is the app's own DOM code (darkening, colour detection, autolink, image
sizing, find-in-thread highlighting, form interception) packed by a small CommonJS bundler that
refuses Node modules (`src/main/frame-script-bundler.ts`). EmailFrame posts `render` and `search`;
the frame posts its height, links, forms, keys, mouse events, the context-menu target, focus and
search match boxes, which the app checks before acting on. `SearchableComponentStore` takes remote
regions (boxes instead of DOM matches). Printing and translation's language detection read the
body through the bridge (`messageFrameContent`). `renderedMessageBodyIntoDocument` gets `{body,
message, iframe}` from plugin API 1.27: `body` is a `MessageBody` (`querySelectorAll`,
`setAttributes`, `insertElement`, `getBoundingClientRect`, `addEventListener` for click and
mousedown, all promises that reject after a re-render; event-handler attributes, scripts, frames,
forms and styles are refused); reading `document` throws, warned once per extension. Link
tracking uses it. The app CSP's `frame-src` is now `tern-message://frame`; `mailspring:` fonts get
`Access-Control-Allow-Origin: *` (fonts load in CORS mode); the chrome-i18n content script ignores
messages that aren't the app window's own strings. Still open: the clipped-message, show-original,
print and quick-preview windows and the composer's quoted body still render in Node-enabled
documents (with TD2's CSP); keyboard events reach the app as re-dispatched copies, as before.

Verified. Every window is an `AppWindow` with `nodeIntegration: true`,
`contextIsolation: false`, no sandbox (`src/main/app-window.ts:124-128`). The message
body is a same-origin `about:blank` iframe in that renderer (`email-frame.tsx:222`). Its sandbox
has no `allow-scripts`, so scripts in mail don't run, and the app CSP blocks injected script, but
remote images, fonts and CSS from mail are decoded in that process. `renderedMessageBodyIntoDocument`
hands extensions the live same-origin document (`email-frame.tsx:155-167`); only
`link-tracking-message-extension.tsx:8` uses it in-tree.

- Cost: one Chromium image, font or CSS parser bug reachable from mail (the libwebp
  CVE-2023-4863 class) is full user compromise with Node and keychain access, with no Chromium
  sandbox in the way.
- Fix: render bodies in a cross-origin, sandboxed out-of-process frame served by main on a custom
  scheme, with a postMessage bridge for height, find-in-thread, link clicks and the
  `renderedMessageBodyIntoDocument` hook. Pairs with TD2's CSP.
- Size: ~700 TS. Before stage 5 bumps the plugin API to 1.27, since that hook's live-document
  contract becomes harder to change afterwards. Not scheduled.

### TD35. `@electron/remote` is the renderer-to-main channel (stages 3 and 6, then any order)

Verified. `src/main/main.js:364` initialises remote and `app-window.ts:180-181` enables it
per webContents. 130 references in 62 files (126 lines in 58 `.ts`/`.tsx`; 35 files in
`features/` and `plugins/`). Ten sites call `getGlobal('application')` for `configPersistenceManager`,
`windowManager`, `autoUpdateManager`, `applicationMenu`, `openUrl` and `isQuitting`
(`src/shared/config.ts:17-19`, `src/window/app-env.ts:898-902`, `src/window/window-event-handler.ts:311`, `:368`,
`src/components/sheets/sheet-toolbar.tsx:182`, `update-notification.tsx:36`, `unsubscribe-service.ts:145`). Every
`AppEnv.config.set` is a sync remote call (`config.ts:747`), as is the initial read (`:735`). About
20 renderer-side `showMessageBoxSync` calls go through remote, including `AppEnv.showErrorDialog`
(~75 call lines). The exposed HTML that actually lands in the main document is pasted or dropped
composer HTML (sanitised by hand because of this, `composer-editor.tsx:287-289`,
`uneditable-plugins.tsx:81-85`), plugin UI, and sanitizer gaps.

- Cost: each remote access is a sync IPC round trip; a sync dialog also blocks main, so one window's
  error freezes the others. Any renderer code, plugins included, can drive any main-process
  object. Remote is deprecated upstream and must be rechecked on every Electron bump. The refactor
  removes only KeyManager's use (stage 6) and `database-store.ts:41` goes with SQLite after stage
  10; new main-process services would get wired the same way.
- Fix: grow stage 6's typed main/renderer IPC module (supervisor, KeyManager facade) into the one
  renderer API: `ipcRenderer.invoke` handlers for dialogs (sync wrapper only where a return value
  is read), `openUrl`, `isQuitting`, window count, menu, updater. Move Config writes onto an explicit
  channel while stage 3 rewrites `ConfigPersistenceManager`; T11 keeps renderer config synchronous,
  so that channel stays `sendSync`. Ban `@electron/remote` for new code with `no-restricted-imports`,
  and drop `remote.enable` at zero. Full `contextIsolation` and `sandbox` then need a plugin API
  decision (plugins `require` Node directly), which nothing addresses yet; start with
  composer and popout windows.
- Size: M–L, ~60 files, mostly mechanical; Config and `showErrorDialog` are the delicate parts.
- Progress (stage 3, 2026-10-18): Config's initial read and writes go over `ConfigChannel`
  (`src/shared/config-channels.ts`, `sendSync`); `.eslintrc` bans `@electron/remote` imports and
  requires outside an allowlist of the 56 files that still use it. Dialogs, `openUrl`,
  `isQuitting`, window count, menu and updater remain.
- Status (2026-10-21, `next`): no code in `src/`, `plugins/` or `static/` uses remote; windows
  reach main through `src/platform/app-api.ts` (handlers in `src/main/ipc/app-api.ts`), and lint
  bans it there with no exemptions. `remote.initialize()`/`enable()` stay for v1 plugins that load
  it until plugin API 2.0; contextIsolation and sandbox are still open.

### TD36. Store graph held together by the barrel and require order (before stage 6)

Verified. 77 non-spec files in `src/` import the plugin barrel `mailspring-exports`, which lazily
requires those files back (`src/plugin-api/mailspring-exports.js:1-48`). About 38 function-scope
`require('./…')` calls break cycles (e.g. `src/flux/models/account.ts:150`, `:220`;
`message-store.ts:92`; `query-subscription-pool.ts:110`). The barrel resolves through
`app-module-path` (`src/window/app-env.ts:70`), and `.eslintrc` lists it under `import/core-modules`, so
`import/no-cycle` never sees these cycles. 27 `load()` entries (24 stores plus DraftEditingSession,
DraftFactory, MessageBodyProcessor; `:129-155`) are touched on nextTick (`:234-240`) in every window,
so composer, popout and onboarding build the whole graph; 13 `isMainWindow()` checks in
`src/flux/stores` are the only per-window control. `task-queue.ts:7-16` records a production crash
(`Rx.Observable.fromQuery is not a function`) caused by require order.

- Cost: load-order crashes recur whenever a module is required earlier than expected, and stages
  1, 5, 6 and 8 add EngineConnection, the `Engine` export, capabilities and a new transport to this
  graph. Secondary windows pay startup time and memory for stores they don't use.
- Fix: ban `mailspring-exports` and `mailspring-component-kit` inside `src/**` with
  `no-restricted-imports` and rewrite the 77 imports as relative. Drop the barrel from
  `import/core-modules` and enable `import/no-cycle` as a warning baseline, then ratchet. Replace
  the nextTick eager resolve with a per-window-type store list called from AppEnv, so new
  engine-backed stores declare where they run.
- Size: M (77 mechanical rewrites, ~100-line store list, spec fallout). Overlap: the refactor
  only touches the lazy getters for `deprecate()`.

### TD37. `AppEnv` is an undeclared plugin API (stage 6)

Verified. `src/window/app-env.ts` (1,002 lines, ~90 members) mixes geometry, dialogs, error reporting,
config, packages, themes, commands, spellchecker, menus and the engine bridge. Plugins use it
heavily (`AppEnv.config` 257 uses, `showErrorDialog` ~75, `savedState` 47, `commands` 31), but the
refactor's Tier A–D lists cover exports only. Stage 6 moves supervision out of `MailsyncBridge`, yet
nine built-in sites call `AppEnv.mailsyncBridge` (`events/lib/main.tsx:43`,
`main-calendar/lib/core/calendar-view.tsx:212`, `:1210`, `account-error-notif.tsx`,
`offline-notification.tsx`, `preferences-account-details.tsx`, `src/flux/tasks/event-rsvp-task.ts:168`,
`destroy-event-task.ts:39`, `syncback-event-task.ts:217`, `:282`), and third-party plugins can too.

- Cost: third-party plugins break at stage 6 or 10 without the refactor's own one-minor-release
  deprecation window; tasks reaching a renderer bridge through a global make the per-window split
  hard to test.
- Fix: declare the supported subset (config, commands, keymaps, menu, themes, dialogs, window-type
  checks, `reportError`) in the plugin API section and a `PublicAppEnv` type. Keep
  `AppEnv.mailsyncBridge` as a `deprecate()` shim forwarding `sendSyncCalendarNow` to `Engine`.
  Give tasks an injected engine handle.
- Size: S (type file, ~40-line shim, a docs paragraph). Docs fold into stage 5.

### TD38. Deltas fan out unbatched through four event systems (stage 6; rx-lite after stage 8)

Verified. Actions use Reflux (TD24), stores their own emitter (`src/plugin-api/mailspring-store.ts`),
queries rx-lite 4.0.8 (deprecated since 2016, 23 files, `src/plugin-api/mailspring-observables.ts`),
Atom-era code event-kit (16 files). `DatabaseStore.trigger` fires once per incoming delta record
(`src/flux/engine-bridge.ts:499`, `:524`).

- Cost: four subscribe and cleanup idioms; Rx monkey-patching caused the TaskQueue crash (TD36). A
  delta burst (initial sync, bulk label change) fans out synchronously per record, and stage 6
  rewires exactly this path.
- Fix: coalesce `DatabaseStore` triggers per microtask or frame in stage 6's per-window delta path.
  After stage 8, swap rx-lite for plain callbacks, or maintained RxJS only where operators are used.
- Size: S for batching; M for rx-lite.

### TD39. Folder identity is a path hash (stage 3, before stage 14)

Verified; downstream effects inferred. Folder sync matches LIST output only by
`hash(accountId:path)` (`crates/tern-core/src/ids.rs:121`, `crates/tern-sync/src/background.rs:814-830`).
After a rename, in Tern or on the server, the old row is unused, its messages are detached and the
folder removed (`:883-930`), a new id refetches headers from scratch, and orphans relink only if
the walk reaches them within 24 hours. `RenameFolderTask` keeps the old id with the new path
(`crates/tern-tasks/src/folders.rs:113-131`), which the next listing discards.

- Cost: renaming a large folder or Gmail label triggers a full header resync and thread churn.
  Anything storing folder ids (rule targets, per-category settings, soon engine settings and rules)
  points at a dead id.
- Fix: before deleting unused folders, pair each with a created folder of the same UIDVALIDITY or
  with the rename `RenameFolderTask` recorded, and keep the id with the new path. Longer term,
  random stable ids with path as a column.
- Size: ~1 week with Gmail labels and scenarios.
- Status (2026-10-18, roadmap/client-next): done for renames; random ids are later. Folder sync
  matches rows to LIST by path, and before removing a row the listing no longer shows, pairs
  it with a newly listed path of the same kind: by a rename `SyncbackCategoryTask` recorded
  (`_State` `folderRenames-<accountId>`, written before the RENAME, kept an hour, children
  follow their parent, either direction), else by a UIDVALIDITY (with UIDNEXT not going
  backwards) checked against the newest three stored copies, whose ids must match at the same
  UIDs under the new path; a value several folders share is settled by those messages, a
  folder with no stored copies pairs only on a value no other mailbox of the account has
  (stored rows and this pass's STATUS), and a label pairs only on a unique value. A paired row keeps its id, role and `localStatus`, so no
  header is fetched again. Labels record their UIDVALIDITY in `localStatus` (25 STATUS per pass
  until each has one; stripped from thread snapshots). The task finds its row by path, keeps the
  id and renames children locally. After a rename the X-GM-LABELS name on the copies and the
  thread snapshots (and their `ThreadSearch.categories`, which `in:` reads) are rewritten in
  chunks, queued in `_State` `folderRenameRefresh-<accountId>`
  in the same transaction so a quit resumes it. A new folder at a path whose hash a renamed row
  holds gets `hash(accountId:path#n)` (`tern_core::ids::free_folder_id`). Code in
  `tern_sync::folder_identity`, `Background::pair_renamed`, `tern_tasks` folders.rs. Desktop:
  `FocusedPerspectiveStore` swaps in the renamed category in place, and `ThreadListStore` keeps
  its data source (scroll position, open thread) for a perspective with the same type,
  accounts, category ids and roles. Scenarios `folder-renamed-in-tern`,
  `folder-renamed-on-server`, `folder-replaced-with-same-uidvalidity`,
  `folder-replaced-empty-with-shared-uidvalidity`, `gmail-label-renamed`; conformance probe `probe_rename_keeps_uidvalidity` (Dovecot keeps
  UIDVALIDITY, UIDs and children across RENAME). Open: whether Gmail keeps a label's
  UIDVALIDITY and bumps MODSEQ on rename is unrecorded; servers that change UIDVALIDITY on
  RENAME pair only through the task's record (the id survives, headers are refetched); mail
  rules' `valueName` fallback is not refreshed after a rename; random stable ids with path as a
  column are later.

### TD40. MessageStore reloads every body on each delta (before stage 8's flip)

Done (roadmap/client-next, stage 8). MessageStore reads a thread without bodies and then the
expanded messages' bodies, patches Message deltas in place, and opens at most three unread
messages expanded.

Verified; cost inferred. Any Message or Thread delta touching the focused thread, except a single
draft change, reruns `findAll(Message).where(threadId).include(body)`
(`src/flux/stores/message-store.ts:121-158`, `:279-312`). Opening an unread thread queues mark-read,
whose deltas cause one or two more full reloads; labels, stars and metadata do the same.
`_expandItemsToDefault` (`:330-345`) expands every unread message, so a 100-message unread list
thread mounts 100 EmailFrames.

- Cost: jank and memory spikes on long or unread threads; after stage 8, each refetch is a socket
  round trip carrying every body as JSON.
- Fix: patch changed messages from the delta objects and keep loaded bodies; fetch bodies only for
  expanded messages missing them; cap default expansion (last unread plus last three) with "expand
  all".
- Size: ~150 TS. Stage 8 is the natural place if the call becomes one `query` with `include`.

### TD41. Every thread row subscribes to ComponentRegistry three times (before stage 8)

Done (roadmap/client-next, stage 8). `InjectedComponent` and `InjectedComponentSet` share one
registry listener (`ComponentRegistry.listenShared`) and `AccountColorBar` one AccountStore
listener; checked in `spec/components/shared-store-listeners-spec.tsx`, not yet in devtools.

Inferred: the warning's source isn't confirmed in devtools. `BaseStore` caps listeners at 250
(`src/plugin-api/mailspring-store.ts:96-104`), so the "251 trigger listeners" warning comes from a
`BaseStore` subclass. Every `InjectedComponent` and `InjectedComponentSet` adds its own
`ComponentRegistry.listen` (`src/components/injected-component.tsx:86`,
`injected-component-set.tsx:116-119`), and each thread row has three
(`features/thread-list/lib/thread-list-columns.tsx:74`, `:140`, `:155`, narrow `:203`,
`:221`), plus one AccountStore listener per row from `AccountColorBar`
(`thread-list-participants.tsx:156`). ListTabular renders `innerHeight/itemHeight + 4` rows
(`list-tabular.tsx:311-321`), ~49 on a 1600px window, so the count scales with window height.

- Cost: console noise that hides real leaks; every plugin activate re-renders hundreds of
  components; subscribe churn while scrolling skews stage 8's scroll timing.
- Fix: confirm with `$m.ComponentRegistry._emitter.listenerCount('trigger')` or
  `--trace-warnings`. Then one registry listener with a version counter feeding a React context
  (or a `useSyncExternalStore` hook); pass account colour through row props. Keep the cap.
- Size: ~80 TS.

### TD42. ThreadSearch is one append-only FTS row per thread (before stage 9)

Verified. Each body fetch appends up to 5,000 chars to the thread's row with no already-indexed
check and no cap (`crates/tern-store/src/search.rs:56-101`, called from
`crates/tern-sync/src/mail_processor.rs:683-687`). Bodies purged after 14 days (`worker.rs:559-571`)
and fetched again on open (`foreground.rs:160`, `background.rs:560`) are appended twice.
`Message::after_remove` (`message.rs:650-658`) never touches ThreadSearch, so text of removed
messages stays searchable. Category changes UPDATE the whole row (`thread.rs:541-547`) and FTS5
re-tokenises all of it. `porter unicode61` (`schema.rs:50`) stems English for every language and
indexes an unspaced CJK run as one token, so only its prefix matches.

- Cost: write amplification grows with thread length; deleted mail stays findable; search is poor
  for non-English users. Stage 9 is about to build the grammar on these columns.
- Fix: per-message external-content or contentless FTS5 keyed by Message rowid
  (subject/from/to/body), inserted when the body is stored and deleted in `after_remove`; threads
  via `SELECT DISTINCT threadId`; categories filtered through ThreadCategory. Consider a trigram
  table, or `unicode61 remove_diacritics 2` without porter, for non-Latin text. Backfill as a TD6
  migration. The port keeps `porter unicode61` for parity; stage 9
  moves only the parser.
- Size: ~1–1.5 weeks, ~700 Rust plus backfill. Stage 7's plain `.search(text)` should target it too.

### TD43. Transient task errors revert the user's action (before stages 11 and 14)

Done (roadmap/client-next, stage 11, 9899c06d1, 825d791ed). `tern-tasks` retries the remote
phase of ChangeUnread/Starred/Folder/Labels, SyncbackCategory, DestroyCategory and
ExpungeAllInFolder after a retryable error: the task stays `remote` with `_remoteAttempts` and
`_nextAttemptAt` in its data, 5 s doubling to 5 min, up to 8 attempts, then fails as before. A
move being retried keeps its optimistic placement and its `_sa` hold, and flag changes don't
rescan until the last attempt. The foreground worker skips a task until its next attempt.
Scenario `flag-change-retried-after-connection-drop`.

Verified; the flag-revert via rescan is inferred. Only `Error::Database` leaves a task queued
(`crates/tern-tasks/src/lib.rs:287-290`, `:353-379`); any other error, retryable or not, marks it
complete with the error. Moves abandon the optimistic placement and flag changes rescan folders
(`crates/tern-tasks/src/messages.rs:614-625`, `:637-645`, `:685-689`). This is the C++ "no automatic
retries" rule, kept on purpose.

- Cost: a connection that drops mid-task (Wi-Fi flap, server cutting a long MOVE, STORE or EXPUNGE,
  or a dead idle socket on servers without IDLE) snaps archive, delete, mark-read or label changes
  back. The after-sleep case mostly reconnects first (`foreground.rs:188-212`). A headless engine
  (stage 11) or rules running in the engine (stage 14) have nobody watching to redo them.
- Fix: for idempotent classes (ChangeUnread/Starred/Folder/Labels, Syncback/DestroyCategory,
  ExpungeAll) keep `remote` status on retryable errors with an attempt count and next-attempt time
  in task data, back off (5 s to 5 min, ~8 attempts), keep the `_sa` hold, fail permanently when
  attempts run out. `SendDraftTask` keeps its guards (see TD33). Metadata syncback is already
  covered by the outbox in `docs/backend-plan.md:229-233`.
- Size: ~300 Rust plus 3–4 harness scenarios.

### TD44. Any non-SyncError stops the account for good (before or with stage 11)

Done (roadmap/client-next, stage 11, 0b6095890, 825d791ed). `Error::is_transient` (retryable
SyncErrors, SQLite BUSY/LOCKED/IOERR/FULL, io errors) keeps the worker loops going with their
backoff; only other errors stop the account, and `serve`'s restart policy (5 s doubling to 5 min,
5 stops in 5 minutes gives up, auth failures wait for the client) restarts it.

Verified. `is_retryable` is true only for a `SyncError` with `retryable` set
(`crates/tern-core/src/error.rs:353-355`). A Database error (`SQLITE_BUSY` after the 60 s busy
timeout, disk full, I/O error) or a serde or io error calls `ctx.fail`
(`crates/tern-sync/src/lib.rs:96-110`), and `serve` never restarts the account
(`apps/engine/src/serve.rs:15-19`). In C++ these crashed the process and the client's CrashTracker
restarted it.

- Cost: a long write elsewhere (VACUUM, reset) or a briefly full disk silently stops sync. Stage 11's
  "5 crashes in 5 minutes" policy would burn its budget on a BUSY burst.
- Fix: treat `SQLITE_BUSY`/`LOCKED`/`IOERR`/`FULL` and io errors as retryable in the worker loops,
  with backoff; keep corrupt and schema errors fatal; feed only fatal errors to the restart policy.
- Size: ~80 Rust plus a test that injects BUSY.

### TD45. No fuzzing of untrusted-input parsers (before stage 11)

Done (roadmap/client-next, stage 11, 375b3ad63, 825d791ed, 6fed85590). `fuzz/` has
cargo-fuzz targets for the IMAP response parser, `parse_message`, rfc2047, HTML flatten, iCal and
vCard, run nightly for 5 minutes each (`.github/workflows/fuzz.yml`; not run locally, no nightly
toolchain here). The literal length uses saturating arithmetic. A message whose processing
panics is quarantined (`quarantine-<folderId>-<uidvalidity>` in `_State`, up to 1000 UIDs) and
skipped, so the account keeps syncing; this works in `serve`, where account threads run under
`catch_unwind`. Scenario `ingest-panic-quarantines-uid` (serve) makes one message panic through
`TERN_ENGINE_TEST_PANIC_SUBJECT` and checks the quarantine key, the other messages and that the
account keeps running.

Inferred: no concrete panic found. The IMAP parser and envelope code
(`crates/tern-imap/src/parser.rs`, `envelope.rs`), MIME (`crates/tern-mime/src/mime.rs`, `header.rs`,
`flatten.rs`), `crates/tern-core/src/rfc2047.rs` and DAV (`crates/tern-dav/src/ical.rs`, `vcard.rs`)
slice buffers by computed offsets on server- and sender-controlled input. For example
`parser.rs:571` computes `p + n` for a literal length unchecked and relies on the transport cap
(`transport.rs:28`). No cargo-fuzz targets or proptest exist. A panic on an account thread stops
the account (`crates/tern-sync/src/runtime.rs:201`, `apps/engine/src/modes.rs:225-235`).

- Cost: one malformed message or response stops an account, the header fetch hits it again on
  every restart, and stage 11's restart policy loops until it gives up. Remote-triggerable denial
  of service.
- Fix: cargo-fuzz targets for the IMAP response parser, `parse_message`, rfc2047, HTML flatten,
  iCal and vCard in nightly CI; checked arithmetic on literal and offset paths; quarantine a UID
  whose processing panicked (record it in the folder's `localStatus` and skip it).
- Size: 2–3 days for targets and CI; ~150 Rust for quarantine.

### TD46. Monthly blocking VACUUM, WAL never truncated (stage 11)

Done (roadmap/client-next, stage 11, 2a1024ebd). No monthly VACUUM; a new database is created
with `auto_vacuum = INCREMENTAL`, every connection sets `journal_size_limit` to 64 MB, and
after an hourly cleanup that deleted rows the engine runs `incremental_vacuum(2048)` and
`wal_checkpoint(TRUNCATE)`; `reset_account` does the same instead of a VACUUM. Existing databases
are not converted (that needs one full VACUUM); they still get the WAL truncation.

Verified; WAL growth inferred. Every 30 days `migrate` runs a full VACUUM before the app starts
(`crates/tern-store/src/migrate.rs:82-95`), needing twice the database size free, and
`reset_account` runs one synchronously (`:106`). No connection sets `journal_size_limit` or
`auto_vacuum` and nothing checkpoints with TRUNCATE (`schema.rs:114-128`).

- Cost: multi-minute launches on big databases, failed launches on full disks, GBs of WAL left
  after initial sync.
- Fix: `auto_vacuum = INCREMENTAL` (the next VACUUM converts) with `incremental_vacuum(N)` in the
  hourly cleanup; `journal_size_limit` ~64 MB and `wal_checkpoint(TRUNCATE)` after big passes; drop
  the monthly full VACUUM or run it idle inside `serve`.
- Size: ~80 Rust. With stage 11, which moves migrate into `serve`.

### TD47. Provider presets: dead entries, three copies, unbounded MX lookup (stage 12)

Verified. `features/onboarding/lib/mailcore-provider-settings.json` still matches
Facebook (fb.com mapped to O365), MobileMe and Openmailbox; the 1064-domain table is an old ISPDB
snapshot; tiles say "Gmail or G Suite", "Office 365", "FastMail" (`account-providers.tsx:6`, `:12`,
`:88`); `apps/web/src/features/settings/accounts/presets.ts` is a third copy. `dns.resolveMx` has no
timeout, and the code's own comment says ~30 s (`onboarding-helpers.ts:65-76`).

- Fix: when stage 12 moves discovery into the engine, keep one `tern-providers` data file regenerated
  from current Thunderbird ISPDB with dead entries removed (after TD4's cleartext pass); desktop
  and web render tiles from `account.discover`; 5 s MX timeout.
- Size: 1–2 days.

### TD48. Three 12/24-hour rules; English 12-hour reply attribution (before stage 13)

Verified. 12/24h comes from `core.workspace.use24HourClock` (`src/utils/date-utils.ts:400-500`), moment's
`LT` (`src/components/time-picker.tsx:144`, `main-calendar/lib/core/calendar-helpers.tsx:377`), or
hard-coded `h:mm A` in eight places (`calendar-event-popover.tsx:971-976`,
`calendar-drag-utils.ts:495-496`, `event-search-bar.tsx:209-214`,
`src/components/propose-time-popover.tsx:21`, `sidebar-participant-profile.tsx:35`, …).
`Message.formattedDate()` is `MMM D YYYY, [at] h:mm a` and `replyAttributionLine()` builds
"On …, X wrote:" in English despite a comment saying it's localised
(`src/flux/models/message.ts:430-439`); both go into every reply and forward
(`draft-factory.ts:239`, `:247`, `:263`). 43 files import moment.

- Cost: 24-hour users see 12-hour times in calendar popovers, and every reply carries an English
  12-hour line whatever the language. Stage 13 ports DraftFactory to `draft.create`, making this
  protocol behaviour for CLI, MCP and web drafts.
- Fix: one `formatTime`/`formatDate`/`formatDateTime` module on `Intl.DateTimeFormat` with the app
  locale and hour-cycle pref, used by the eight sites and `LT` checks. Before stage 13, specify the
  attribution line (localised template, locale and 24h from settings) and store template and
  locale in the engine settings store, as the engine does for resurfacing texts.
- Size: ~150 TS plus ~15 call sites.
- Status (2026-10-18, stage 13, roadmap/client-next): done. `src/shared/date-format.ts` has
  `formatTime`, `formatShortTime`, `formatDate`, `formatDateTime` and `hourCycle` on
  Intl.DateTimeFormat; the hard-coded `h:mm A`/`h:mma` sites, the `LT` checks and renders
  (time picker, calendar helpers, popover, drag preview, search bar, agenda, day and week
  views, participant profile, propose-time popover) and `date-utils.ts`'s own hour-cycle reads go
  through it. One rule: the 24-hour setting on is 24 hours, off is the locale's own clock (a
  decision to confirm; before, off meant 12 hours in `date-utils.ts` and the locale's clock in
  `LT`). The attribution line is `localized('On %1$@, %2$@ wrote:')` with the date from
  `formatQuoteDate`, and the engine builds the same line from `composing.quoteTexts`, which the
  settings mirror writes in the user's language and clock (date pattern from Intl's parts).
  moment is still imported for date arithmetic and the remaining date-only formats.

### TD49. Very large sync and send functions (stage 13; with TD39 and TD55)

Verified. `sync_now` (~404 lines, `crates/tern-sync/src/background.rs:265`),
`sync_folders_and_labels` (~216, `:739`), `perform_remote_send_draft` (~288,
`crates/tern-tasks/src/send.rs:176`) and `sync_folder_uid_range` (~189, `worker.rs:238`) are
line-for-line C++ ports mixing many invariants. tern-sync has ~36 unit tests; most coverage is the
Python harness.

- Cost: TD39, TD55 and stage 13 all land in these bodies; once the C++ A/B oracle goes (stage 10),
  the code is the only reference.
- Fix: extract per-step functions with unit tests as part of the stage that next edits them (send
  path in stage 13; `sync_now` with TD39 or TD55), not as a standalone pass.
- Status (2026-10-18, stage 13): not done. Stage 13 didn't edit `perform_remote_send_draft`
  (`draft.send` queues the existing `SendDraftTask`; the shared finalisation is
  `tern_sync::expiration_executor::finalise_draft`), so the split stays with the next stage that
  changes the send body.
- Status (2026-10-18, TD39): `sync_folders_and_labels` lost the LIST and helper-folder step
  (`list_remote_folders`) and role assignment (`assign_roles`); rename pairing is
  `Background::pair_renamed` over unit-tested functions in `tern_sync::folder_identity`.
  `sync_now` is unchanged.

### TD50. EventSearch and ContactSearch updated by an UNINDEXED column (before stage 18)

Done (roadmap/client-next, stage 18). Event and Contact keep their FTS rowid as `searchRowId`
(`crates/tern-store/src/models/search_row.rs`) and update and delete by it. A row written before
that is found by `content_id` once, on its next save, so no migration was needed; a
`searchRowId` that names another model's row is not trusted.

Verified (EXPLAIN). `content_id` is declared UNINDEXED (`crates/tern-store/src/schema.rs:31`, `:66`)
and every event update and delete and non-mail contact update uses `WHERE content_id = ?`
(`models/calendar.rs:346-364`, `models/contact.rs:203-221`), giving a full virtual-table scan per
row. Inherited.

- Cost: a CalDAV pass every 15 minutes or a CardDAV sync costs O(changes × table size) under the
  write lock; stage 18 adds engine-side edits on top.
- Fix: use the model's rowid as the FTS rowid (or store it on the model, as Thread does) and
  update by rowid; one TD6 migration rebuilds both tables.
- Size: ~120 Rust plus migration.

### TD51. Three timezone databases, two copies of ical.js (after stage 18)

Verified. moment and moment-timezone 0.6 (34 and 14 files) bring their own tz data; the lock has
ical.js 2.2.1 at the top level and 1.5.0 nested under ical-expander; `src/utils/calendar-utils.ts:78`
registers VTIMEZONEs only on 2.2.1. The engine uses chrono-tz.

- Cost: zone fixes have to be made in two places, and desktop and engine can disagree on
  occurrences.
- Fix: don't port off moment piecemeal. After stage 18 moves expansion to `events.expand`, delete
  ical-expander and ical.js from the desktop, then move display formatting to TD48's module.

## Later

### TD52. React 17 with APIs React 19 removes

Verified. React and react-dom 17.0.2 (`apps/desktop/package.json:59`, `:61`), react-test-renderer 17 and
`@testing-library/react` 12 to match. `findDOMNode` 148 hits in 69 files (e.g.
`src/components/fixed-popover.tsx:78`, `:81`), string refs 37 in 17 files, `this.refs` 44,
`ReactDOM.render` 12 in 8 files (`src/window/app-env.ts:839`, `modal-store.tsx:23`, `popover-store.tsx:22`,
`create-page-for-form.tsx:203`), `createRoot` 0. No `UNSAFE_` lifecycles remain. 244 class
components against 7 hook uses.

- Cost: two majors behind; plugin authors target React 17 typings; each new UI (MCP pane,
  integrations, stage 8 engine-state banners) adds code on APIs that must be rewritten.
- Fix: step 1, now: codemod string refs to `createRef` and `findDOMNode` to root refs under TD27's
  lint rules (~1 week, ~70 files). Step 2, after stage 8's flip: React 18 with `createRoot`, and
  audit store-listener `setState` callers for automatic batching, so async reads and batching
  changes aren't debugged together. React 19 only after TD53.

### TD53. Composer on a 2019 Slate fork pinned to personal GitHub commits

Verified, except the fork's last-commit date. `slate` `github:bengotow/slate#cd6f40e8`,
`slate-react` `#0.45.1-react` (0.21.12), `@bengotow/slate-edit-list#b868e108`
(`apps/desktop/package.json:32`, `:67`, `:73`; lock `:160`, `:8076`, `:8180`), plus a fourth personal
fork, `debug` from `emorikawa/debug` (lock `:2549`). Immutable 3 exists only for it.
`src/flux/stores/draft-store.ts:76-84` calls `require('slate').resetMemoization()` every five
minutes because Slate's caches leak. slate-react uses `findDOMNode`. The refactor keeps the composer,
DraftEditingSession and DraftChangeSet in Electron, so nothing else
retires this.

- Cost: no upstream fixes (IME, accessibility, paste, lists); installs depend on forks staying up
  and npm audit skips git deps; blocks React 19.
- Fix: now, vendor the four forks as tarballs under `packages/` (S). After stage 13, replace the
  editor with TipTap/ProseMirror behind the composer-editor boundary and the `ComposerExtension`
  hooks, using stage 13's HTML corpus (quoting, signatures, sanitising) as the fidelity check (L,
  3–6 weeks; composer-editor is ~3–4k lines plus plugins).

### TD54. Four packaging paths, dead grunt and C++ leftovers

Verified; cold-start cost inferred. `build/build.js:146-163` transpiles each file with
`transpileModule` and no bundler. The asar unpack list (`:241-268`) names `node_modules/spellchecker`, which isn't a
dependency.
`script/grunt` and `grunt.cmd` point at a missing `build/Gruntfile.js`. deb and rpm are hand-rolled
(`script/mkdeb`, `mkrpm`), Windows uses Squirrel (`electron-winstaller` 5.4.0, effectively
unmaintained), and the feed is empty (`autoupdate-manager.ts:23`).

- Cost: each format has its own signing quirks and gets tested only at release; the GitHub-release
  feed `backend-plan.md:49` mentions would be built against these formats; hundreds of small files
  load from the asar at start.
- Fix: delete the grunt scripts and stale unpack entries now. Then electron-builder (deb, rpm,
  AppImage, NSIS or MSIX, dmg, zip), electron-updater with a GitHub provider, fuses (TD28) in
  `afterPack`. esbuild for main and renderer can come later.
- Size: tiny now; 1–2 weeks for the builder move, after stage 2 (tern-mcp bundling) and stage 10
  (C++ entries gone), before an update feed exists.

### TD55. STATUS on every folder every 2 minutes, IDLE only on Inbox

Verified; throttling risk inferred. Each background pass sends one STATUS per selectable folder in
sequence, then sleeps 120 s (`crates/tern-sync/src/background.rs:305-311`,
`crates/tern-core/src/constants.rs:60-61`, `:111`); IDLE covers Inbox or All Mail only
(`foreground.rs:218-303`); errors sleep a fixed 120 s with no backoff or jitter. No LIST-STATUS
(RFC 5819) or NOTIFY (RFC 5465).

- Cost: hundreds of round trips every 2 minutes on big accounts; filtered folders lag up to 2
  minutes; a hosted engine multiplies it.
- Fix: LIST-STATUS when advertised; NOTIFY where supported (Dovecot, Cyrus) with STATUS fallback;
  skip folders unchanged for N passes; exponential backoff with jitter. Needs fakeimap support.
- Size: ~1 week. After stage 10 retires the C++ traffic A/B.

### TD56. Error reporting would leak a stable device id and unscrubbed data

Done (launch security pass, ceed467b3). Nothing reports errors off the machine: the Sentry
reporter and getmac are gone, `ErrorLogger` only logs, and the crash reporter keeps dumps on disk
(`uploadToServer: false`). The GitHub issue prefill with the sync error JSON was unreachable and
is deleted. If error reporting comes back, it needs the fix below: a random install id stored in
config, and a scrub pass over the event and `extra` (the `report-error` IPC still takes any JSON
from Tern's windows).

Inferred: nothing is sent today. The DSN is empty
(`src/error-logger-extensions/sentry-error-reporter.js:8`). Once set, events carry an unsalted
SHA-256 of the MAC via getmac as `user.id` (`:175-186`), the same scheme as Mailspring's, so
installs link across apps, plus `extra` sent verbatim (`:201-238`) with home paths, addresses and
hostnames. The `report-error` IPC accepts arbitrary JSON from renderers
(`src/main/application.ts:867-913`). "Contact support" already pre-fills a public GitHub issue
with the account's full `syncError` JSON (`notifications/lib/items/account-error-notif.tsx:36-49`).

- Fix: a random install id stored in config, no getmac (TD24). A scrub pass (emails, home paths,
  hostnames, token-like strings) for Sentry and the GitHub prefill, and show the user the issue body
  before opening it.
- Size: ~100 TS. Before any DSN or backend crash ingestion is turned on; the GitHub prefill part
  can go now.

### TD57. A LESS compiler kept for v1 third-party stylesheets

Verified. The app's stylesheets are plain CSS. LESS (`less-cache`, `src/packages/compile-cache-less.ts`)
is compiled only for packages in the profile's packages folder: community LESS-variables themes,
whose tokens come from `static/styles/compat/tokens.less` (`ThemeManager.themeTokensCSS()`), v1
plugins that ship `styles/*.less`, and the preview colours of such themes
(`src/packages/theme-preview.ts`). Their import surface is `static/styles/compat/less`
(`ui-variables`, `ui-mixins`, `base/*`, `mixins/*`) plus `static/styles/compat/base/ui-variables.less`;
both are kept in step with `static/styles/tokens/defaults.css` and `static/styles/variables.css` by
hand, with theme-tokens specs checking that `compat/tokens.less` sets every `variables.css` property.
`tests/packages/stylesheets.test.ts` fails on a `.less` anywhere else in the app (`static/styles`
outside `compat`, `src/`, `plugins/`, `themes/`).

- Cost: a 2018-era compiler and a second copy of the core values to keep in step; LESS's
  arithmetic on slashes and its colour functions only work on the literal values.
- Fix: at plugin API 2.0, with the rest of v1: delete `static/styles/compat`, `compile-cache-less.ts`,
  the `.less` branch of `ThemeManager.cssContentsOfStylesheet()` and of `stylesheetsIn()`, the LESS
  path in `theme-preview.ts`, and the `less-cache` and `less` dependencies.

## Checked and dropped

No item was refuted outright. These sub-claims were checked and removed or corrected above:

- Scripts in email HTML run with Node access: no, the body iframe has no `allow-scripts`
  (`email-frame.tsx:222`). TD34 and TD35 now state the real exposure.
- The clipped-message window loads remote images regardless of the setting: no, it goes through
  `processUnclipped` and the autoload extension (`message-body-processor.ts:132-197`). Only quick
  preview ignores the setting (TD2).
- github-contact-card is on by default: no, the shipped template `default-profile/config.json`
  disables it; the schema default `[]` only applies when the key is missing. Gravatar is the real
  default leak (TD3).
- Engine files at 0644 are readable by other users today: mostly no, the desktop chmods the config
  dir to 0700 (`config-persistence-manager.ts:37-45`). It matters once the engine creates its own
  directory (TD12).
- Branch-ref slate-react re-resolves on every install: no, the lock pins a SHA. Contributors
  without SSH keys are blocked: mostly no, npm falls back to HTTPS for public GitHub repos (TD9,
  TD53).
- Fuses must land before stage 11: backwards; stage 11 moves secrets out from under Electron's
  keychain item, so fuses should land as soon as stage 2 removes the RunAsNode dependency (TD28).
- After-sleep stale sockets revert tasks: mostly no, the foreground loop reconnects before running
  tasks; mid-task drops are the real trigger (TD43).
- fix-backlog F2 recommends Rebuild Cache: no, it notes Rebuild is currently needed to re-enable
  Important (TD32).
