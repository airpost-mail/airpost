# AI providers: summaries, reply suggestions, triage

2026-10-18 (tern-ai). Bring-your-own-key cloud models and local models through Ollama,
used for thread summaries, reply suggestions and inbox triage. Section 10's recommendations
were agreed with the user on 2026-10-18. Desktop paths are relative to `apps/desktop/app/`.

## 0. Decisions

| # | Decision |
|---|---|
| A1 | Provider calls run in the engine, never in Electron, the browser or the backend. A new crate `crates/ms-ai` holds the providers, prompts and output parsing, transport-free and store-free like `ms-integrations`; the engine adds `ai.*` socket methods and builds the model input from its own database. Desktop, CLI, web (through the gateway) and tern-mcp share one implementation. |
| A2 | Three provider kinds: **OpenAI-compatible** (`/v1/chat/completions`; covers OpenAI, OpenRouter, Groq, Mistral, Gemini's compatible endpoint, LM Studio, llama.cpp, vLLM), **Anthropic** (`/v1/messages`), **Ollama** native (`/api/chat`, `/api/tags`). Ollama gets its own kind because its OpenAI endpoint ignores `num_ctx` and silently truncates long threads to the model's default window. |
| A3 | Keys follow integrations I3 and T13: the engine reads a stored provider's key from its SecretStore as `integration:ai-<providerId>`, and requests carry no key. The key belongs to the provider's server: a `settings.set` that removes the provider or changes its kind or base URL deletes `integration:ai-<id>` before it writes, and is refused if it can't. The desktop saves and forgets it with `secrets.set` / `secrets.delete` on an engine that lists `ai-keys`, and changes no key while the engine is down; stage 11's `credentials.import` moved the keyset's `integration-ai-<providerId>-key`. With an older engine (one before AI6 lists `account-setup` and `ai-triage` too) the desktop keeps it in `KeyManager` under that name and sends it per request. |
| A4 | Non-secret AI config (providers, models, feature switches, account opt-in, limits) is engine-owned, in stage 3's `settings` store under `ai.*` keys. Until stage 3 lands it lives in the desktop's `config.json` (`core.ai.*`) and goes with each request, next to the key. |
| A5 | Off by default. Each account is opted in separately, each feature is switched on separately, and the engine enforces both on every call, whichever client asks. Mail content goes only from the engine to the provider host the user configured; never through `apps/backend`. |
| A6 | The model never acts. Output is text shown to the user or inserted into an editable draft, plus triage values from a fixed vocabulary. No tool use, no sending, no IMAP changes. Model output is rendered as plain text (no links, no images), so an injected prompt can't exfiltrate through a URL. |
| A7 | Streaming over the socket: the client picks a `streamId`, the engine sends `ai.delta` notifications to the calling connection only, then the normal reply with the full text. `ai.cancel {streamId}` and closing the connection stop the provider request. |
| A8 | Logs carry provider kind, host, model, status, token counts and timing. Prompts, completions, keys and addresses are never logged and never part of a cache key in clear (integrations I6). |

## 1. Where calls run

```
desktop / tern / web (via gateway) ──socket──▶ engine ── ai_api.rs ──▶ ms-ai ──HTTPS──▶ provider
                                                 │  reads threads, messages, bodies (Store)
                                                 └─ triage worker on new inbox mail
```

- `crates/ms-ai`: reqwest (rustls on ring), serde, thiserror, tracing, the same stack as
  `ms-integrations`, but reqwest's async client on a private one-worker tokio runtime
  that callers block on, so `ai.cancel` can drop a request that is still waiting for its
  first token. Depends on `ms-integrations` for `Secret`, its URL redaction, the TTL cache
  and the transport-error mapping, nothing else. No store access: callers pass a `MailContext` (below).
- `apps/engine/src/ai_api.rs`: method routing, context building from the read-only Store,
  account and feature checks, the stream registry, the triage worker, usage counters.
- tern-mcp gets no generation tools (the MCP client is already a model); it can read triage
  results through `metadata.list` like any other plugin metadata.

## 2. The crate (`crates/ms-ai`)

```rust
let ai = Ai::new()?;                                        // or with_options(Options { … })
ai.models(&ProviderConfig)                                  -> Result<Vec<ModelInfo>>
ai.test(&ProviderConfig)                                    -> Result<TestReport>   // host, model, latency
ai.summarize(&ProviderConfig, &MailContext, &SummaryOpts, &mut dyn Sink) -> Result<Completion>
ai.suggest_replies(&ProviderConfig, &MailContext, &ReplyOpts)            -> Result<Vec<Suggestion>>
ai.draft_reply(&ProviderConfig, &MailContext, &DraftOpts, &mut dyn Sink) -> Result<Completion>
ai.triage(&ProviderConfig, &MailContext)                    -> Result<Triage>
```

- **`ProviderConfig`** `{id, kind: "openai"|"anthropic"|"ollama", url?, apiKey?: Secret,
  model, contextTokens?, maxOutputTokens?}`. Default URLs: `https://api.openai.com/v1`,
  `https://api.anthropic.com`, `http://127.0.0.1:11434`. `local: bool` is derived (loopback or
  a private address in `url`) and drives the privacy note. Plain `http://` is refused unless
  the host is local or a private name (one label, `.local`, `.lan`, `.home.arpa`,
  `.internal`).
- **Wire**: OpenAI SSE `data:` lines to `[DONE]`, `stream_options.include_usage`; Anthropic
  SSE `content_block_delta` / `message_delta` with `anthropic-version: 2023-06-01`; Ollama
  NDJSON with `options.num_ctx` set to `contextTokens` (default 8192). `models()` uses
  `GET /v1/models` (OpenAI, Anthropic) and `GET /api/tags` (Ollama).
- **`MailContext`**: subject, participants as display names (addresses only when
  `includeAddresses`), messages oldest first with date, sender name, and plain-text body
  with quoted replies dropped (`ms_mime::flatten_html_with` without blockquotes) and
  everything after a `-- ` signature line cut, plus the
  account owner's name so "you" is resolved. Without `includeAddresses`, addresses in
  bodies, subjects and file names become the person's name or `[address]`; body lines that
  would read as the block's own headers are escaped. The builder fits a token budget (4
  chars per token for Latin, Greek and Cyrillic, 1 per char for other scripts;
  `contextTokens` minus output and prompt, the output limit lowered to fit a small window):
  the newest messages whole, older ones cut from the middle, and a note to the model when
  something was left out.
- **Prompts** live in the crate as versioned constants (`PROMPT_VERSION` is part of cache
  keys). Mail text goes in a delimited data block, with the instruction that it is content
  to describe, not instructions to follow. Output language follows the thread.
- **Triage output** is JSON validated against a fixed schema: `category` one of
  `needs-reply`, `important`, `newsletter`, `notification`, `promotion`, `other`;
  `needsReply: bool`; `reason` (one short sentence). Anything else is `other` with
  `reason: null`. OpenAI-compatible and Ollama use their JSON response formats; Anthropic
  gets the schema in the prompt and as structured output (`output_config.format`), because
  current Claude models reject a prefilled assistant turn with a 400.
- **Errors**: one `AiError`, serialized like `IntegrationError` (`{key, message, retryable,
  status?, retryAfter?}`), with keys `not-configured`, `not-enabled` (account or feature
  off), `auth`, `rate-limited`, `quota-exceeded`, `offline`, `bad-request`,
  `model-not-found`, `context-too-long`, `provider-error`, `cancelled`, `unsupported`.
  Timeouts: 10 s connect, 30 s to the first token, 120 s total (local models are slow).
- **Cache**: in memory, per kind, SHA-256 of provider id, model, prompt version and the
  context's message ids and body hashes. Summaries 24 h / 500, suggestions 1 h / 200. Draft
  replies and errors aren't cached. Nothing persisted in this version.

## 3. Socket methods

All take `accountId`; the engine checks the account is opted in and the feature is on
before it reads any mail. The provider is a stored one (`core.ai.providers`, section 4's
settings), picked by `provider` id or, without one, the one the feature uses, with its key from
the secret store (`integration:ai-<id>`). A local client may instead send `config`, the provider
config; one without `apiKey` gets the stored key of the provider with that `id` only when it
names the same kind and base URL, so the stored key never goes to another host (Preferences'
Test and Load Models send the form this way). Nor does rewriting `core.ai.providers`: a
`settings.set` that removes a provider or points it at another kind or base URL deletes its key
first (`ai_api::forget_unbound_keys`). The desktop sends `provider` to an engine that lists
`ai-keys` (`feature::AI_KEYS`), and `config` with the keyset's key to an older one.
The thread methods also take `settings` (`ms_ipc::AiSettings`): `{accounts: [accountId],
features: {summary, reply, triage}, includeAddresses?}`, each feature `true` or `{enabled: true,
…}`, from the desktop's `core.ai.*`; it wins over the stored switches. Without either, or with
the account or feature missing, the call is `not-enabled`; a provider that is not stored, or
that needs a key the store doesn't hold, is `not-configured`.

| Method | Params | Result |
|---|---|---|
| `ai.models` | `config` \| `provider` | `[ModelInfo {id, name, contextTokens?}]` |
| `ai.test` | `config` \| `provider` | `{host, model, local, latencyMs}` |
| `ai.summarize` | `accountId`, `threadId`, `streamId?`, `refresh?` | `{text, model, cached, usage?}` |
| `ai.suggestReplies` | `accountId`, `threadId`, `count?` (default 3), `streamId?` (for `ai.cancel` only; no `ai.delta`) | `[{label, text}]` (short one-liners) |
| `ai.draftReply` | `accountId`, `threadId`, `instruction?`, `suggestion?`, `streamId?` | `{text, model, usage?}` (plain text; the composer converts) |
| `ai.triage.run` | `accountId`, `threadIds?` \| `sinceDays?` | `{jobId, queued}` |
| `ai.triage.list` | `accountIds?`, `category?`, `limit`, `offset?` | `{threadIds, total}` |
| `ai.cancel` | `streamId` \| `jobId` | `{}` |
| `ai.usage` | `days?` (default 30) | `[{day, provider, feature, inputTokens, outputTokens, requests}]` |

Notifications, added to the routing table of desktop-client-plan 3.1:

| Notification | Filter | Backlog | Dedup |
|---|---|---|---|
| `ai.delta {streamId, text}` | the connection that sent `streamId` | no | in order; the final reply has the full text |
| `ai.triage.progress {jobId, done, total, stopped?}` | the connection that called `ai.triage.run` | no | `jobId`; the last one has `done == total` or `stopped: true` |

Errors: `-32602` for params that don't deserialize (and for a `streamId` already streaming on
the connection); an `AiError` is engine code `-32011` "AI request failed" with the serialized
error as `data`. Unknown accounts and threads are `-32001` and `-32004`, a thread with nothing
that may be sent (drafts, Spam, Trash, encrypted mail only) is `unsupported`. The model calls
run on the `ai` worker lane (8 threads, 32 queued; `-32012` past that); `ai.cancel` and
`ai.usage` are answered at once, in order. `ai.delta` needs no `subscribe`.

**Relay policy** (`crates/ms-ipc/src/policy.rs`): every `ai.*` method is `Open` except that a
relayed connection may never pass `config` (nor the interim `settings`, which would let it opt
accounts in), only `provider`, admin scope or not. Otherwise a remote device could make the
engine send requests, and mail, to any URL, including hosts on the user's LAN.
Both rules go in `ms_ipc::policy::relayed_request_allowed` (the method classes and the
`config` refusal), which the gateway and the engine both enforce, not only in the engine's
socket code. Until then `ai.*` are unknown names, so `LocalOnly` (admin devices only).
`apps/gateway/tests/notifications.rs` (tern-web, main b9f6b53e5) is switched to assert the
`config` refusal once `ai.*` are `Open`. The gateway already routes `ai.delta` and
`ai.triage.progress` only to the WebSocket whose engine connection started the request, in
order, and closing a tab closes that connection, which cancels the provider request (A7).

**Features and capabilities.** `hello.features` gains `ai`. Stage 5's `capabilities` gains
`ai: {summary, reply, triage}`, each `{available, provider?, local?, reason?}`, judged by the
engine from the stored settings and keys, which is what a call without `config` and `settings`
(a relayed one, `tern ai`, background triage) can do: `not-set-up` (the feature off, no account
opted in, no stored provider or model), `needs-engine-key` (a provider that needs a key while
`integration:ai-<id>` holds none), `quota-exceeded` (the daily cap), and for triage only
`rate-limited` and `quota-exceeded` during a pause. The desktop words them as integrations-plan
section 1 does ("Set up a model in Preferences > AI", "Add your Anthropic API key in Preferences
> AI"). `capabilities.changed` follows `settings.set` of `core.ai.*`, `secrets.set` /
`secrets.delete` of an `integration:ai-*` name, a `credentials.import` that carried one, and
triage pauses. Engines before AI6 sent `summary` and `reply` as `{available: true, credentials:
"request"}`, leaving them to the client. `ms_ipc::Capability` gained `local`.

## 4. Triage

- **Trigger.** An engine thread (`thread::AI`, spawned through `WorkerContext::spawn` per
  account) takes `mail.new` events for opted-in accounts into a bounded queue (1000), and
  `ai.triage.run` jobs. One request in flight per provider; a `rate-limited` or
  `quota-exceeded` error pauses the queue until `retryAfter` (or 15 minutes; clamped to 1 s
  to 24 h) and sets the capability reason; the daily token cap holds it until midnight or
  until the cap is raised. Default limit 200 threads per account per hour. The switches and
  the provider are read again before every request, so turning triage off or changing its
  provider applies to queued work and running jobs within a second. A job that ends early
  (cancelled, account stopped, triage turned off, provider gone) sends a last progress with
  `stopped: true`.
- **Input.** The thread's latest inbound message in full (budgeted) and the subjects and
  senders of the rest, so the cost per mail stays small.
- **Result.** Thread metadata under plugin id `tern-ai-triage`, value `{category, needsReply,
  reason, messageId, model, v: PROMPT_VERSION, at}`, written through the local metadata path
  the expiration executor uses. That gives every client deltas and `metadata.list` for free
  and needs no schema change. The plugin id is in a local-only list in `ms-net`'s metadata
  backend, so a connected Tern server never receives it (A5): each device triages its own
  mail. A new inbound message re-triages the thread; a user's manual change sets
  `source: "user"` and is never overwritten.
- **`ai.triage.list`** reads opted-in accounts' inbox threads with that metadata
  (`json_extract` over the Thread's metadata, newest first) so a "Needs reply" view works on
  today's read API without stage 7's `query`. A needs-reply value lapses once the user sent
  mail on the thread after the last mail came in (or, for `source: "user"`, after its `at`);
  the desktop chip follows the same rule.
- Backfill is manual: "Triage inbox" in the desktop runs `ai.triage.run {sinceDays: 7}`
  (capped at 200 threads), with progress.

## 5. Privacy

| Feature | What leaves the engine | When |
|---|---|---|
| Summary | the thread's subject, sender names, dates and stripped plain-text bodies (budgeted) | the user clicks Summarize |
| Reply suggestions / draft | the same, plus the user's instruction and their display name | the user asks for suggestions or a draft |
| Triage | the latest inbound message (budgeted), the other messages' subjects and senders | every new inbox thread in an opted-in account with triage on |

- Addresses are replaced by display names unless the user turns on "Include email
  addresses". Attachments are never sent, only their file names.
- Skipped always: Spam and Trash, drafts, and messages that are S/MIME or PGP encrypted.
- Every UI that sends mail to a model names the host ("Sent to api.anthropic.com" or "Runs on
  this computer (Ollama)"), and Preferences > AI shows the table above.
- Summaries and suggestions live only in the engine's memory cache; triage values in local
  metadata. Account reset and `engine.resetDatabase` drop both.

## 6. Cost and rate limits

- `ai.usage` reports tokens and requests per day, provider and feature, from provider
  `usage` fields (estimated for Ollama when absent). Counters live in `<config>/ai-usage.json`
  (90 days). No prices: provider tables change too often to ship.
- Optional per-provider daily token cap; past it every call is `quota-exceeded` until
  midnight local time.
- Retries: one retry on `offline` and `provider-error` for non-streamed calls; `rate-limited`
  honours `Retry-After`. Streamed calls are never retried mid-stream.

## 7. UX

**Desktop** (one internal package `internal_packages/ai`, on the `Engine` export of T18):
- *Preferences > AI* (new tab; MCP's "AI assistants" stays in Integrations): a list of
  providers (kind, URL, key with Save Key and Forget Key as in integrations section 2, model
  picker from `ai.models`, Test), which provider each feature uses, feature switches, the
  per-account opt-in list, "Include email addresses", the privacy table and the usage chart.
- *Summary*: a Summarize button in the message list header for threads of 2 or more messages
  (or one long one). The summary streams into a card above the first message, with the model
  name, host and Regenerate. Opening the thread again shows the cached one.
- *Reply suggestions*: in a reply composer, a Suggest button shows up to 3 one-line
  suggestions and a free-text field ("say I'll call Tuesday"). Picking one streams a full
  reply above the quoted text, as one undoable edit. Never sent automatically.
- *Triage*: a small category chip in the thread list (`needs-reply` and `important` only,
  to keep the list quiet), a "Needs reply" item per opted-in account in the sidebar
  (`ai.triage.list`), and a chip menu to correct the category.
- Availability and tooltips come from the capabilities (`useCapability('ai-summary')`).

**Web** (`apps/web/src/features/ai`): the same summary card, Suggest in the reply composer
and triage chips and "Needs reply" list. Settings shows the configured providers and
switches read-only, because keys and provider URLs are local-only (section 3).
`WsEngineClient` (`apps/web/src/engine/ws.ts`) times calls out after 30 s, which is less than
the engine's 120 s AI deadline. Streamed `ai.*` calls pass a longer per-call timeout, which
is part of the minimal wiring.

**CLI**: `tern ai summarize <thread>` (streams to stdout), `tern ai triage [--since 7d]`,
`tern ai usage`.

## 8. Tests

- `ms-ai`: a fake provider on the integrations tests' small HTTP server, speaking all three
  wire formats: scripted text split into chunks, usage fields, each error status,
  `Retry-After`, a slow first token, a stream cut mid-way, malformed JSON for triage. Tests
  check request shape (headers, model, `num_ctx`, no key in logs), budgeting, the triage
  schema fallback and cancellation.
- Engine: `tests/ai_api.rs` over the socket against the fake: opt-in and feature checks,
  `ai.delta` order and routing to the calling connection only, `ai.cancel`, a dropped
  connection cancelling the provider request, relayed connections refused `config`.
- Harness: `ai_triage_new_mail` (quick tier): fake IMAP delivers mail, a Python fake provider
  in `apps/engine/test` answers, the thread gets `tern-ai-triage` metadata; a second scenario
  for the rate-limit pause.
- Desktop specs with a mocked `Engine.request`; web tests on the mock engine.

## 9. Stages and dependencies

| Stage | Work | Depends on | Size |
|---|---|---|---|
| AI1 | `crates/ms-ai`: three providers, streaming, context builder, prompts, triage schema, errors, cache, fake provider tests | none | ~1.5k Rust + tests |
| AI2 | Engine `ai.*` except triage: summarize, suggestions, draft, models, test, cancel, `ai.delta`, relay policy, `hello.features` and capabilities | AI1; stage 5 (worker pool, `capabilities`) | ~700 Rust |
| AI3 | Desktop: Preferences > AI, summary card, reply suggestions; config and keys sent per request | AI2; stage 5 (`Engine` export, CapabilityStore on capabilities) | ~1k TS |
| AI4 | Triage: worker, metadata, local-only plugin id, limits, usage, `ai.triage.*`; desktop chips and "Needs reply"; `tern ai` | AI2; stage 3 (`ai.*` settings) for engine-owned config; stage 11 for keyed providers in the background (Ollama works without) | ~900 Rust, ~400 TS |
| AI5 | Web: summary, suggestions, triage | AI2, AI4; stage 11 (engine-held keys); the gateway relaying `ai.delta` and `ai.triage.progress` (tern-web) | ~600 TS |

Order: AI1 now; AI2 rebased onto stage 5 when it lands; AI3 straight after; AI4 after stage 3
(it can run on Ollama-only until stage 11); AI5 last.

What other sessions need to know (shapes from their plans, not built here):
- tern-client, stage 3: `ai.providers`, `ai.features`, `ai.accounts`, `ai.limits` and
  `ai.includeAddresses` are added to the settings keys of 3.4.
- tern-client, stage 5: `ai.*` are slow methods on the pool; `capabilities.ai` as above.
- tern-client, stage 11: `integration-ai-<id>-key` imported like other `integration-*` keys.
- tern-web: relay `ai.delta` and `ai.triage.progress` to the device that started them;
  stage 7's allowlist doesn't need triage, since `ai.triage.list` covers the view.

## 10. Questions (answered 2026-10-18: all recommendations taken)

1. **One provider, or one per feature?** Recommend: a list of configured providers and a
   choice per feature, so triage can run on Ollama while replies use a cloud model.
2. **Where triage results live.** Recommend thread metadata under `tern-ai-triage`, never
   synced to a Tern server. Alternative: a separate engine database, which costs a schema
   and its own deltas.
3. **Triage on new mail automatically?** Recommend yes, for opted-in accounts with triage on,
   plus a manual 7-day backfill; no automatic backfill.
4. **Fixed triage categories?** Recommend the six in section 2, no custom ones in this
   version.
5. **Preferences placement.** Recommend a new "AI" tab, leaving MCP's "AI assistants" in
   Integrations where you put it.
6. **Web stage AI5.** tern-ai builds it in `apps/web/src/features/ai`, only after tern-web's
   notification relay is on main, and changes nothing else in `apps/web` beyond the minimal
   wiring (route, settings entry, engine method types).
7. **Persisting summaries.** Recommend memory only for now; persist later if regenerating
   costs too much.

## Status

2026-10-18: draft written; the user took all of section 10's recommendations (6 as amended
there). Stages run as one background workflow.

2026-10-18, AI1 done on `roadmap/ai`: `crates/ms-ai` with the three wire formats, streaming
into a `Sink` with `CancelToken`, `MailContext` and its budget, prompts v1, triage and
suggestion parsing with fallbacks, `AiError`, the summary and suggestion caches, timeouts
and one retry for non-streamed calls. `ms-integrations` now exports `RedactedUrl`, its
cache and its HTTP error helpers for it. Tests run a fake provider in all three formats.
Decided while building: usage reaches the engine through `Options::usage_observer` (one
`UsageRecord` per request) so the method results keep the shapes of section 2; when
`contextTokens` / `maxOutputTokens` are unset, cloud providers default to 16384 / 4096
and Ollama to 8192 / 1024; OpenAI's own host gets `max_completion_tokens`, other
compatible servers `max_tokens`.

2026-10-18, AI1 review fixes: thinking and reasoning deltas meet the first-token deadline;
the default output limit is at most a quarter of `contextTokens` and every request lowers
it to fit the window; non-Latin scripts count a token per character; JSON mode is dropped
(and remembered per server) after a 400 naming `response_format` (LM Studio); a 429 is
`quota-exceeded` only for billing or credit, and keeps `Retry-After`; usage is recorded for
accepted requests that fail, are cut or are cancelled; cache hits carry no usage; plain
http only to local hosts; addresses hidden in bodies, subjects and file names; forged
`--- Message` and header lines escaped (prompts v2); answers, stream lines and error and
model-list bodies are size-capped; `RenderedContext` and `Triage` redact `Debug`.

2026-10-18, AI1 gated and merged: fmt, clippy, workspace tests, `cargo deny`, release build
of mailsync, tern, tern-mcp and tern-gateway, harness quick tier on serve (131 passed, 1
skipped). Rust only, so no desktop or web gate. Open: nothing in the engine calls `ms-ai`
yet; the `ai.*` methods, keychain secrets and relay policy come in AI2.

2026-10-18, AI2 on `roadmap/ai`: `apps/engine/src/ai_api.rs` serves `ai.models`, `ai.test`,
`ai.summarize`, `ai.suggestReplies`, `ai.draftReply`, `ai.cancel` and `ai.usage` on `serve`'s
socket (not `sync --socket` or the parent's stdin, like integrations). Thread calls check the
account (opted in through the per-request `settings`) and the feature, then read the
`MailContext` on the connection's reader thread from its read-only database connection:
drafts, hidden messages, messages whose every copy is in Spam or Trash, and S/MIME, PGP/MIME
or inline PGP messages left out; bodies flattened without cited blockquotes, then `>` lines,
the attribution line before them and anything after an Outlook "Original Message" separator
dropped; the snippet when a body isn't fetched; bodies read newest first up to 8 characters
per token of the window, counted without signatures, older ones sent with `body_omitted` so the
model is told; every body checked for inline PGP whatever the budget; participants taken from
the messages kept, not the thread's list (which has every draft's and Spam message's people);
dates in the engine's local zone; the owner from the account's name and address. The model call runs on the new `ai` lane with a sink that sends `ai.delta` to
that connection's queue ahead of the response; each call's `CancelToken` is registered with
its connection, cancelled by `ai.cancel {streamId}` and by the connection closing.
`ai.suggestReplies` is cancellable too (`Ai::suggest_replies_until`), by its optional
`streamId` (no `ai.delta`). ms-ai's runtime now
has one worker thread (named `ai`, so a panic there is contained): on a current-thread
runtime nothing ran hyper's connection tasks between calls, so a cancelled request's socket
stayed open and the provider kept generating. `ai.usage` counts in memory per day, provider
and feature (90 days; the file is AI4). The caches are cleared on `account.reset` and
`engine.resetDatabase`. ms-ipc: the `ai.*` method names (triage included), `feature::AI`,
`code::AI_FAILED`, the params types, `Capabilities.ai`, `Capability.local`, and the relay
policy (all `ai.*` `Open`, `config` and `settings` refused on relayed connections, admin or
not, absent params allowed; tern-web's gateway tests pass unchanged); `AiProviderParams`
redacts `config` in `Debug`. Tests: `apps/engine/tests/ai_api.rs` against
the fake provider (opt-in and feature checks before any read or request, privacy of what is
sent, `ai.delta` order and routing, the cache, suggestions, drafts, models, test, `ai.cancel`,
a dropped connection cancelling the provider request, relayed refusals), unit tests for the
quote stripping, encryption checks, streams and usage, and the ms-ipc policy.

Open after AI2: the engine trusts the request's `settings` until stage 3 adds the `ai.*` keys
(tern-client), and has no stored providers until then and no keys until stage 11, so relayed
devices get `not-enabled` / `not-configured`; `capabilities.ai` can only say "served" until
then. Usage isn't persisted (AI4).


2026-10-18, AI2 gated and merged: fmt, clippy, workspace tests, `cargo deny`, release build
of mailsync, tern, tern-mcp and tern-gateway, harness quick tier on serve (141 passed, 4
skipped). Rust only, so no desktop or web gate. Open as above.

2026-10-18, AI3 on `roadmap/ai`: desktop. `src/ai.ts` (exported as `AI`, plugin API 1.28.0)
reads and writes `core.ai` (`providers`, `features` with a provider id each, `accounts`,
`includeAddresses`), builds ms-ai's provider config with the key from the keyset
(`integration-ai-<id>-key`) and the interim `settings`; it mirrors ms-ai's base URL, host,
`local` and key-required rules. `core.ai` is not mirrored to the engine's settings store
(stage 3's `ai.*` keys). CapabilityStore has `ai-summary` and `ai-reply` (source `ai`, with
`host` and `local`): "Set up a model in Preferences > AI" without a provider or model, "Add your
… API key" for Anthropic or OpenAI's own API without a saved key, "Waiting for engine support"
without `hello.features` `ai`, and the engine's word when it reports a feature unavailable.
`internal_packages/ai`: `ai-client.ts` makes the calls through `Engine.request`, checks the
switches before reading the key, collects `ai.delta` for its own streamId through
`Engine.subscribe`, maps `-32011` data to `AiError` and short messages, and cancels with
`ai.cancel`. Preferences > AI (tab after Integrations): providers with name, URL, key (Save Key,
Forget Key; Test and Load Models use the typed key without saving it), model from `ai.models`
or typed, context window, Test and Remove (which deletes the key); per feature a switch and a
provider; the account opt-in list; Include email addresses; the privacy table. The summary is a
`MessageListHeaders` component for threads of 2 or more sent messages or one of 4000+ characters:
Summarize, the card streaming as plain text (`white-space: pre-wrap`, no HTML), model and "Sent to
host" / "Runs on this computer", Stop, Regenerate (`refresh`), Close; the window remembers the
last 100 summaries by thread and newest message, so reopening a thread shows it without a call.
Suggest is a `Composer:ActionButton` on replies: a popover with up to 3 suggestions and an
instruction field; the draft streams into a preview and, when complete, goes into the body above
the quote and the signature (blank leading lines replaced, typed text kept) as one body change,
escaped. Closing the card, the thread or the popover cancels the call. Specs (36) with a mocked
`Engine.request`: the client, the capabilities, Preferences > AI, insertion, the card and the
suggestions panel.

Open after AI3: the usage chart in Preferences > AI waits for persisted usage (AI4); triage UI is
AI4; per-account and per-feature availability is still the desktop's own check until stage 3.

AI3 review fixes: the reply goes in before the first top-level element that is or holds a
signature, `.gmail_quote_attribution`, `.gmail_quote` or any blockquote (found with DOMParser, so
the composer's serialized body works too), and in plain text above the whole attribution line
(soft-wrapped lines included) past the blank line DraftFactory leaves before the quote. The Suggest
popover names the host and sends nothing until Get Suggestions, Write or a pick; the Suggest
button's title and the text next to Summarize name it too. "Runs on this computer" is loopback
only (`AI.isLoopback`); a private address reads "Sent to 192.168.1.20 (your network)", in the
Test result too. Changing a provider's server (scheme, host or port) deletes its saved key.
Errors: a dropped connection is `engine-disconnected`, -32012 is `engine-busy` (no provider
named), a keyset read failure is `keyset`; `engine-unavailable` only when the engine doesn't serve
`ai.*`. The window's remembered summaries go when summaries are turned off, the account is opted
out or removed, or `account.state` says `removed` (account.reset does that); resetDatabase
relaunches the app.

2026-10-18, AI3 gated and merged: fmt, clippy, workspace tests, `cargo deny`, release build
of mailsync, tern, tern-mcp and tern-gateway, harness quick tier on serve (141 passed, 4
skipped); desktop lint and typecheck clean, the ai package's specs (50) and the specs of the
modules it touches (engine client and events, CapabilityStore, package manager, the other
capability and engine-using packages) pass. No web changes. Open as above.

2026-10-18, AI4 on `roadmap/ai`: triage. Settings: the engine's store takes `core.ai.providers`,
`core.ai.features`, `core.ai.accounts`, `core.ai.includeAddresses` and `core.ai.limits`
(`{triagePerHour?, dailyTokens?: {providerId: n}}`) under `feature::AI_TRIAGE` (`ai-triage`).
They are named after the desktop's config key paths (`core.ai`), so stage 3's mirror carries
them unchanged; that is the smallest change to stage 3's key list, and a deviation from the
`ai.*` names in section 9. A relayed connection may read them but never write them, admin or
not. Every `ai.*` call uses the stored provider and switches when the request has none (a
request's `config` and `settings` still win), so keyless Ollama works with nothing sent per
request; keyed providers still need the key per request until stage 11.
`apps/engine/src/ai_triage.rs`: each account's `aiTriage` thread takes `mail.new` items off its
delta stream and `ai.triage.run` jobs into a bounded queue (1000 new-mail items, 8 jobs), with
one request in flight per provider across accounts, `triagePerHour` (200) per account, the
daily token cap, and a pause on `rate-limited` or `quota-exceeded` until `retryAfter` (15
minutes without one) during which `capabilities.ai.triage` is unavailable (`rate-limited`,
`quota-exceeded`; `needs-engine-key` for a keyed provider, `not-set-up` otherwise). The input is
the thread's latest inbound message (`Bodies::LatestInbound`); the answer is stored as
`tern-ai-triage` thread metadata `{category, needsReply, reason, messageId, model, v, at}`
through `SyncbackMetadataTask`'s local phase. A thread already triaged for its latest inbound
message at this prompt version is skipped, and a value with `source: "user"` is never
replaced. The plugin id is local-only in ms-net (`LOCAL_ONLY_PLUGINS`): never pushed, rebound
or pulled, and ms-tasks skips its remote phase; a reset drops it from the metadata stash.
`ai.triage.run` (`threadIds` or `sinceDays`, at most 200; a local client may send `config`) answers
`{jobId, queued}`, then `ai.triage.progress` goes to the calling connection; jobs outlive the
connection and only it can cancel them (`ai.cancel {jobId}`). `ai.triage.list` pages the
opted-in accounts' triaged inbox threads by category (`needs-reply` also matches `needsReply`).
Usage is persisted in `<config>/ai-usage.json` (90 days) and `core.ai.limits.dailyTokens` caps
each provider per local day. Log lines (`ms_core::logmsg`): `- AI triage stored for thread …`,
`-X AI triage paused for N s: …`, `AI triage resumed`.
CLI: `tern ai summarize <thread> [--refresh]` streams `ai.delta` to stdout, `tern ai triage
[--since 7d | thread ids]` waits for the job's progress (the engine finishes it if interrupted),
`tern ai usage [--days]`; no provider config is sent, so they use the engine's stored providers.
Desktop (plugin API 1.29.0): the mirror carries `core.ai.*` with an engine that lists
`ai-triage` (`writeSettings` keeps `limits`); CapabilityStore has `ai-triage` with the engine's
reasons; `internal_packages/ai` adds the chip (`Thread:MailLabel`, needs-reply and important
only) with a correction menu that files `source: "user"`, "Needs reply" in the sidebar for each
account with triage on (a perspective over `ai.triage.list`, asked again on Thread changes; the
sidebar now skips an extension's null item and rebuilds when an extension registers), and
Preferences > AI gets the triage switch, "Triage inbox" per opted-in account (last 7 days, with
progress and Stop) and a privacy row.
Tests: `apps/engine/tests/ai_api.rs` (stored settings, the daily cap, run/list/no re-ask/user
value kept, the rate-limit pause and capabilities, job cancellation), unit tests in ms-ai,
ms-net, ms-store, ms-ipc and the engine, the CLI's clap tests, the harness `test_ai.py` with a
fake Ollama (`harness/servers/ai.py`): `ai_triage_new_mail`, `ai_triage_rate_limit_pause` and
`tern ai`, and desktop specs (triage, capabilities, the settings mirror).

Open after AI4: background triage with a keyed provider still answers `needs-engine-key`. Stage 11
landed on main during AI4 (rebased onto it): `credentials.import` stores the desktop's
`integration-ai-<id>-key` as `integration:ai-<id>`, but `StoredAi` doesn't read the secret store
yet, so that is the next step, together with dropping the per-request key;
the usage chart in Preferences > AI is not built (`ai.usage` and `tern ai usage` have the data);
`core.ai.limits` has no UI; AI5 (web) waits for tern-web's relay of `ai.delta` and
`ai.triage.progress`.

2026-10-18, AI4 review fixes: `Retry-After` is clamped to 1 s to 24 h (0 no longer loops, a
huge value no longer panics the worker); the daily cap is no longer a gate pause but read from
the settings on each try, so raising it resumes triage within a second and the capability
follows; the cap is checked again under the provider's in-flight lock; the switches and the
provider are read again before every request (A5), and a job or new-mail item uses the
provider the store names then (a local client's `config` stays the job's); `ai.cancel {jobId}`
wakes a worker waiting on a pause or the hourly limit; every job ends with `done == total` or
`stopped: true` (`Feed::close` reports the running and queued jobs), which `tern ai triage`
reports as an error and the desktop's "Triage inbox" as "Stopped before the end."; the
desktop also stops on the engine's `gone`, cancels a job stopped before `ai.triage.run`
answered and drops its listener when Preferences closes. Triage and `Bodies::LatestInbound`
treat the account's aliases as its own (`Account::own_addresses`, shared with ms-sync).
`settings.set` refuses a `core.ai.providers` entry with `apiKey`, and `StoredAi` drops one
stored earlier. `tern ai summarize` strips control characters from model text. "Needs reply"
drops threads the user answered (store filter and chip); the chip has no right-click menu
(the thread list's own opened as well), and the view's threads can be moved, archived and
trashed again.

2026-10-18, AI4 gated and merged: fmt, clippy, workspace tests, `cargo deny`, release build
of mailsync, tern, tern-mcp and tern-gateway, harness quick tier on serve (147 passed, 4
skipped, `test_ai.py`'s 3 among them); desktop lint and typecheck clean, the ai package's specs
and those of the modules it touches (settings mirror, package manager, account sidebar,
capability packages; 114) pass. No web changes. Open as after AI4: the engine reading
`integration:ai-<id>` from the secret store, the usage chart, a UI for `core.ai.limits`, AI5.

2026-10-18, AI5 on `roadmap/ai`: web, in `apps/web/src/features/ai`. Calls carry no `config` or
`settings` (a relayed connection may not), so the engine's stored `core.ai.*` providers and
switches apply; the settings are read-only on the web. The thread summary card, Suggest in the
composer (inserted above the signature and typed text), the triage chip on thread rows, "Needs
reply" in the sidebar (over `ai.triage.list`, skipping removed accounts) and Settings > AI
(read-only providers and switches, 30 days of usage). No "Triage inbox" on the web yet; the mock
serves `ai.triage.run`. Streamed calls collect `ai.delta` by streamId; the model calls
get a 150 s timeout in `ws.ts`. Remembered summaries go on account reset or a provider change;
mapped IPv6 loopback counts as local. The mock engine serves `ai.*` (demo outside tests).
Tests: 4 vitest files in `features/ai/test`.

2026-10-18, AI5 gated and merged: fmt, clippy, workspace tests, `cargo deny`, release build of
mailsync, tern, tern-mcp and tern-gateway, harness quick tier on serve (147 passed, 4 skipped);
web lint, typecheck, tests (378) and build clean. Web only, so no desktop gate. Open as after
AI4: the engine reading `integration:ai-<id>` from the secret store, the usage chart on the
desktop, a UI for `core.ai.limits`; on the web, "Triage inbox".

2026-10-18, AI6 on `roadmap/ai`: engine-held keys. `StoredAi` reads each stored provider's key
from `serve`'s secret store as `integration:ai-<id>` (`ms_secrets::names::ai_provider`;
`credentials.import` maps the desktop's `integration-ai-<id>-key` to it), through a source
`serve` registers (`ai_api::on_secret`), so background triage, relayed calls and `tern ai` run on
keyed providers. A local client's `config` without a key gets the stored one only for the same
provider id, kind and base URL. `capabilities.ai` is judged by the engine for all three features
(`needs-engine-key` only while the secret is missing); `secrets.set` / `secrets.delete` of an
`integration:ai-*` name and a `credentials.import` carrying one send `capabilities.changed`.
Desktop: with an engine that lists `ai-keys` (added in review, below), Preferences > AI saves,
finds and forgets keys with `secrets.set`, `secrets.has` and `secrets.delete` (dropping a keyset
copy), a server change or Remove deletes the engine's key (and keeps the provider if the engine
can't), the thread calls and "Triage inbox" send `provider` instead of `config`, and Test and
Load Models send the form's config with only a typed key. The main window moves keys a build
before AI6 saved to the keyset after `credentials.import` ran (it imports non-account keys once)
into the engine. Older engines keep the per-request path. The capability reason reads "Add your
… API key in Preferences > AI". Tests: `engine_held_keys_serve_triage_and_relayed_calls` in
`apps/engine/tests/ai_api.rs` (file-backed store via `TERN_ENGINE_KEY_FILE`, the key imported on
stdin, a fake Anthropic provider: triage and a relayed summary send the stored key, another host
gets none, delete and set change the capabilities, and no message on either socket carries the
key), engine unit tests for the key lookup and the capabilities, desktop specs for both paths
(client, Preferences, triage, capabilities).

AI6 review fixes: the engine lists `ai-keys`, and the desktop keeps keys in the engine only
with it (an engine before AI6 lists `account-setup` and `ai-triage` too, so the earlier gate
moved keys to engines that never read them). `settings.set` deletes the key of a provider it
removes or re-points at another kind or base URL before writing, and refuses the write when the
delete fails on a readable store. `secrets.delete` answers `{}` when the store can't be opened or
read (`Registry::forget_secret`): nothing in it can be read to send, and Preferences could
otherwise never remove or re-point the provider. The call that uses up a provider's daily cap
sends `capabilities.changed`, and while one is capped an `aiCapReset` thread sends another once
the local day ends. The desktop's `forgetKey` and `hasSavedKey` throw while the engine is down,
so Preferences keeps the old server or the provider; with an older engine `forgetKey` also
drops `integration:ai-<id>` where `secrets.*` exists. Tests:
`a_provider_moved_or_removed_loses_its_key` and the cap notification in
`daily_token_cap_refuses_calls_until_midnight` (`apps/engine/tests/ai_api.rs`), unit tests for
`unbound_keys` and `forget_secret`, desktop specs for an engine without `ai-keys` and for a
disconnected one. Left as is: a store unreadable when a provider is removed and readable later
still holds its key; it is used again only if a provider with that id is stored again.

Open after AI6: the usage chart in Preferences > AI, a UI for `core.ai.limits`; on the web,
"Triage inbox", and its `needs-engine-key` text still says to use a local model or the desktop.

2026-10-18, AI6 gated and merged: fmt, clippy, workspace tests (1185), `cargo deny`, release
build of mailsync, tern, tern-mcp and tern-gateway, harness quick tier on serve (147 passed, 4
skipped); desktop lint and typecheck clean, the ai package's specs (81) pass, and the full
desktop suite has 3208 passing and 4 failing, the same 4 as on main: "stops listening once
disposed" in `triage-spec.tsx` (passes alone; an earlier spec leaves a `gone` listener) and
three `SystemTrayIconStore` focus specs. No web changes. Open as after AI6.
