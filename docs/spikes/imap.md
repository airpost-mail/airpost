# IMAP spike: async-imap or our own command layer

Plan section 10 asks whether async-imap 0.12 (over imap-proto 0.17) can carry the
`ImapSession` trait. The driver is `crates/ms-imap/examples/async_imap_spike.rs`; it starts
the harness servers through `crates/ms-imap/tests/support/serve.py` (the fake with the
`dovecot` and `yahoo-messagelimit` personalities, Dovecot 2.3.21 and Cyrus 3.6.1 in Docker)
and prints what async-imap exposes.

```sh
CARGO_TARGET_DIR=target/p1-i cargo run -p ms-imap --example async_imap_spike
```

## Findings

| Question | async-imap 0.12 | Evidence |
|---|---|---|
| Unsolicited responses (VANISHED, EXPUNGE, FETCH during IDLE) | Partly. Everything not tied to the running command goes to `Session::unsolicited_responses`, a bounded(100) channel filled with `try_send`, so a burst the caller has not drained yet is dropped without notice. `VANISHED` has no variant and arrives as `Other`, and the `VANISHED (EARLIER)` a `UID FETCH … (CHANGEDSINCE n VANISHED)` asked for lands in the same channel instead of in the command's result (Cyrus: `Vanished { earlier: true }` then `Vanished { earlier: false }`, both only in the channel). | Spike output on all three servers; `src/parse.rs` `handle_unilateral`, `src/client.rs:355` |
| MESSAGELIMIT response codes (RFC 9738) | No. `uid_fetch` consumes the tagged OK; its response code is never returned. imap-proto has no `MESSAGELIMIT` code and leaves it in the free text (`Outcome { code: None, information: "[MESSAGELIMIT 50 11] …" }`). On the fake `yahoo-messagelimit` server `UID FETCH 1:*` returned 50 of 60 messages with nothing to say it was cut short. | Spike Q2 |
| Lenient ENVELOPE parsing | No. A display name with an unescaped quote, or an address list written `(NIL)`, fails the whole response; async-imap turns any parse error into an I/O error that ends the stream, so one bad message fails the FETCH and the connection. 8-bit bytes inside a quoted string do parse. | Spike Q3; `src/imap_stream.rs` `decode` |
| IDLE interrupt from another thread | Yes. Dropping the `StopSource` from another OS thread ends `wait_with_timeout` with `ManualInterrupt` (~500 ms after the drop was scheduled, on every server). A server that sends data in the same instant wins the race (Cyrus answered with a late `* 0 RECENT`). | Spike Q4 |
| `C`-prefixed tags (Coremail) | No. `IdGenerator` always produces `A0001`…; there is no way to change the prefix. libetpan's 163 workaround (`mailimap.c:2461, 2550`) tags `C1`, `C2`, … | `src/types/id_generator.rs` |
| COMPRESS=DEFLATE | Yes, behind the `compress` feature: `Session::compress` wraps the stream, and a compressed session against Cyrus selected and fetched normally. Neither the fake nor the harness Dovecot (no `imap_zlib`) advertises it. | Spike Q6 (Cyrus) |

Beyond the six questions, imap-proto 0.17 cannot parse two responses the login sequence
depends on: `* NAMESPACE ((\"\" \"/\")) NIL NIL` and `* XLIST (…) "/" "Inbox"` are parse
errors, and `[NOMODSEQ]` is not a known code. async-imap also has no hook for the raw bytes on
the wire, which the verbose `sent`/`recv` transcript needs.

## Decision

async-imap fails four of the six questions, so the session does not use it (plan section
10). imap-proto does not get kept as the parser either: it cannot parse NAMESPACE or XLIST,
has no MESSAGELIMIT or NOMODSEQ codes, and fails a whole response on one malformed field, so
using it would mean a second parser for the responses it lacks plus a fallback for the ones
it rejects.

`ms-imap` therefore owns the whole stack behind the unchanged `ImapSession` trait:

- **Transport** (`transport.rs`): tokio TCP, rustls with `rustls-platform-verifier` for TLS
  and STARTTLS, `async-compression` raw DEFLATE for COMPRESS, every byte passed to the
  `ProtocolLogger`.
- **Framing and parsing** (`parser.rs`): one response at a time, literals included, read
  into a generic token tree (atoms, strings, literals, lists, bracketed sections). Each
  response kind is interpreted from the tree, so a field that does not have the expected
  shape costs that field or that response, never the command. A response that cannot be
  tokenised at all is logged as `error-parse` and skipped.
- **Commands** (`session.rs`, `command.rs`): libetpan's tag style (`1`, `2`, … and `C1`, `C2`,
  … after a Coremail greeting), mailcore's command shapes, and every untagged response
  routed through one handler, so VANISHED, EXISTS and response codes such as MESSAGELIMIT
  are seen whichever command they arrive with.
- **IDLE**: a `tokio::select!` over the socket, the 28-minute timer and `IdleInterrupt`
  (a `tokio::sync::Notify` that another thread can fire).

Dependencies dropped from `ms-imap`: `async-imap`, `imap-proto` (both stay as
dev-dependencies of the spike example only). The workspace entries can go once nothing else
uses them.

## Result

The stack above is in `crates/ms-imap/src/` (`transport.rs`, `parser.rs`, `envelope.rs`,
`session.rs`); command shapes follow libetpan and mailcore so transcripts line up with the C++
engine's. How the six questions came out:

| Question | Session behaviour |
|---|---|
| Unsolicited responses | Every untagged response goes through one handler. Plain `VANISHED` is collected per folder (keyed by the folder being selected during SELECT) for `take_vanished`; `VANISHED (EARLIER)` is returned with the `sync_by_uid` that asked for it. An unsolicited FETCH for a UID the command also fetches is replaced by the later one. |
| MESSAGELIMIT | `[MESSAGELIMIT n uid]` on the tagged or any untagged OK sets `hit_message_limit`; `message_limit()` reads `MESSAGELIMIT=` from CAPABILITY. |
| Lenient ENVELOPE | Unescaped quotes are resolved by where a quote can end; an address with more than four parts and an envelope with more than ten items are rejoined; `(NIL)` address lists are skipped; an untagged response that cannot be tokenised is logged as `error-parse` and skipped. FETCH responses without an ENVELOPE trigger mailcore's FullHeaders retry, which then sticks for the session. |
| IDLE interrupt | `IdleInterrupt` from any thread; an interrupt that arrives before IDLE ends the next one right after the continuation. |
| `C` tags | `C1`, `C2`, … after a greeting containing `Coremail System IMap Server Ready`. |
| COMPRESS | Enabled after login when advertised (Cyrus); the transcript shows the decompressed protocol. |

Gate runs (`crates/ms-imap/tests/session.rs`, ignored by default because they need Python
and Docker; `tests/scripted.rs` covers the malformed cases against an in-process server):

```sh
CARGO_TARGET_DIR=target/p1-i cargo test -p ms-imap --test session -- --ignored --test-threads 4
CARGO_TARGET_DIR=target/p1-i cargo test -p ms-imap --test scripted
```

Each of the nine probes of `apps/engine/test/conformance/probes.py` plus a scripted
CONDSTORE / QRESYNC / IDLE session passes against the fake (`dovecot` personality), Dovecot
2.3.21 (`qresync`) and Cyrus 3.6 (`fastmail`), along with Cyrus `default-ns` (an `INBOX.`
namespace) and the fake's gmail, netease-coremail, yahoo-messagelimit, courier,
hmailserver, proton-bridge and inbox-only personalities, XOAUTH2, implicit TLS and STARTTLS.
Server differences the tests had to allow for: Cyrus gives each message of one STORE its own
modseq, answers a CHANGEDSINCE fetch with FETCH lines for messages it then reports VANISHED,
and can wake IDLE with a `\Deleted` flag change before the EXPUNGE; Dovecot and the fake
answer STATUS of the selected folder from the connection's stale view until a NOOP.
