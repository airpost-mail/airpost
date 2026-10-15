# DAV spike: libdav or our own protocol layer

Plan section 6 lists `libdav` 0.11 for CalDAV/CardDAV, to be evaluated in the DAV phase
(P1-D). The question was whether `crates/ms-dav` can build the DAVWorker port on top of it.
The evaluation read libdav 0.11.0's source (`~/.cargo/registry/src/*/libdav-0.11.0`) against
what `MailSync/DAVWorker.cpp` does on the wire.

## Findings

| Question | libdav 0.11.0 | Evidence |
|---|---|---|
| Fits the calContacts thread (plan D2, 4.1) | No. `WebDavClient<C: HttpClient>` is async over a `tower_service::Service` returning hyper `Response<Body>`. The DAV sync code (sync-collection paging, fallbacks, per-server quirks) is sequential and written blocking, like the C++; using libdav would make all of it async for this one dependency. | `src/common.rs:19` `trait HttpClient`, `src/dav/mod.rs:292` |
| `sync-collection` REPORT (RFC 6578) | No. It names `DAV:sync-collection` and `DAV:sync-token` (`src/names.rs:64-71`) and asks for `supported-report-set`, but has no request that sends the REPORT or parses its changes. That REPORT is the main path of the C++ contacts and calendar sync. | `grep -rn sync-collection src` |
| Truncated results (507 in the body or as the HTTP status) | No handling. Cyrus/Fastmail page sync-collection that way (`DAVWorker.cpp` sync loop). | no `507` in the source |
| Raw hrefs | No. Response hrefs go through `normalise_percent_encoded` (`src/xmlutils.rs:286`, `src/dav/mod.rs:742`) and request paths through `strict_percent_encoded`. The engine stores the server's href and echoes it back byte for byte in multigets and deletes, and Google's `%40` paths must survive unchanged. | |
| Error shapes | Its own `thiserror` enums. The engine needs `SyncError` keyed `Invalid Response Code: N` with `<url> RETURNED <body>` debuginfo, `etag-conflict` for 412, and libcurl's texts for transport errors (plan D7). Every call site would need a mapping layer. | |
| DNS | Pulls in the `domain` 0.12 crate for SRV/TXT. The engine already uses hickory (plan D3), so this is a second resolver. | `Cargo.toml` `[dependencies.domain]` |
| Licence | ISC, compatible. Not a blocker. | `Cargo.toml` |
| Retry-After / 429, Bearer vs Basic, If-Match writes, server quirks | Not provided; would be written on top in any case. | |

What libdav does well (discovery, PROPFIND helpers, typed property names) is the small part
of DAVWorker. The parts that carry the engine's behaviour (sync-collection paging, the
etag/time-range fallbacks, the per-server quirks, the error shapes) would all be written
outside it, against an async API that does not match the thread model.

## Decision

libdav is not adopted. `crates/ms-dav` owns the protocol layer:

- **Transport** (`transport.rs`): a `Transport` trait with one blocking `send`; the real one
  is reqwest's blocking client over rustls (ring), redirects off as with curl, an HTTP/1.1-only
  client for servers that need it. Tests script responses through the same trait.
- **Runtime contract**: `DavClient`, `ReqwestTransport` and `DnsResolver` block the calling
  thread and may be called with or without a tokio runtime entered (plan 4.1 has network
  threads run a current-thread runtime). reqwest's blocking client and a nested `block_on`
  both panic inside a runtime, so when `Handle::try_current()` succeeds each HTTP call and DNS
  lookup runs on a scoped thread (`runtime.rs`). A P5 worker can call ms-dav from inside its
  `block_on`; it should not do so from a task that shares the thread with other async work,
  since that work stalls for the length of the request.
- **Auth**: `DavAuth::Bearer` holds a token provider called before every request, as
  `getAuthorizationHeader` asked the token manager each time. The `.well-known` probes carry
  no credentials, as in the C++ (`PerformExpectedRedirect`), since they also go over plain
  http.
- **XML** (`xml.rs`): `roxmltree` (MIT/Apache-2.0, already a libdav dependency, no C code)
  for multistatus parsing, namespace-aware, keeping hrefs raw. Request bodies are the C++
  strings byte for byte (`requests.rs`).
- **Discovery** (`discovery.rs`): SRV/TXT through hickory, a known-provider table, then
  `.well-known`, current-user-principal, the home set and the collections.
- **Client** (`client.rs`): ctag/etag/sync-collection REPORTs with 507 paging, multiget in
  chunks of 90, If-Match PUT/DELETE (412 is `etag-conflict`, 404/410 on delete is success; a
  412 to an If-Match DELETE is checked with a Depth 0 PROPFIND and counts as gone on 404/410,
  because sabre/dav answers 412 when the resource no longer exists, RFC 9110 §13.1.1),
  429/503 Retry-After handling, Bearer or Basic auth, and the server profile flags from the
  DAVWorker comments (`profile.rs`).
- **Content**: `calcard` (Apache-2.0/MIT) decodes iCalendar and vCard values; `ical.rs`
  extracts the Event row fields with chrono-tz for TZIDs, and `vcard.rs` keeps the C++'s
  lossless line model for editing group cards.

The `ms-dav` Cargo.toml no longer lists `libdav` or `quick-xml`. The workspace entries can be
dropped by whoever owns the root manifest.
