# AI providers: summaries, reply suggestions, triage

2026-10-18 (tern-ai). Bring-your-own-key cloud models and local models through Ollama,
used for thread summaries, reply suggestions and inbox triage. Section 10's recommendations
were agreed with the user on 2026-10-18. Desktop paths are relative to `apps/desktop/app/`.

## 0. Decisions

| # | Decision |
|---|---|
| A1 | Provider calls run in the engine, never in Electron, the browser or the backend. A new crate `crates/ms-ai` holds the providers, prompts and output parsing, transport-free and store-free like `ms-integrations`; the engine adds `ai.*` socket methods and builds the model input from its own database. Desktop, CLI, web (through the gateway) and tern-mcp share one implementation. |
| A2 | Three provider kinds: **OpenAI-compatible** (`/v1/chat/completions`; covers OpenAI, OpenRouter, Groq, Mistral, Gemini's compatible endpoint, LM Studio, llama.cpp, vLLM), **Anthropic** (`/v1/messages`), **Ollama** native (`/api/chat`, `/api/tags`). Ollama gets its own kind because its OpenAI endpoint ignores `num_ctx` and silently truncates long threads to the model's default window. |
| A3 | Keys follow integrations I3 and T13: until stage 11 the desktop keeps them in `KeyManager` as `integration-ai-<providerId>-key` and sends them per request; stage 11's `credentials.import` already moves `integration-*` keys, after which the engine reads `integration:ai-<providerId>` from its SecretStore and requests carry no key. |
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
  `ms-integrations`, but reqwest's async client on a private current-thread tokio runtime
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
before it reads any mail. `config` is the provider config with key until stages 3 and 11,
then omitted (the engine uses its stored config; `provider` picks a configured one by id).

| Method | Params | Result |
|---|---|---|
| `ai.models` | `config` \| `provider` | `[ModelInfo {id, name, contextTokens?}]` |
| `ai.test` | `config` \| `provider` | `{host, model, local, latencyMs}` |
| `ai.summarize` | `accountId`, `threadId`, `streamId?`, `refresh?` | `{text, model, cached, usage?}` |
| `ai.suggestReplies` | `accountId`, `threadId`, `count?` (default 3) | `[{label, text}]` (short one-liners) |
| `ai.draftReply` | `accountId`, `threadId`, `instruction?`, `suggestion?`, `streamId?` | `{text, model, usage?}` (plain text; the composer converts) |
| `ai.triage.run` | `accountId`, `threadIds?` \| `sinceDays?` | `{jobId, queued}` |
| `ai.triage.list` | `accountIds?`, `category?`, `limit`, `offset?` | `{threadIds, total}` |
| `ai.cancel` | `streamId` \| `jobId` | `{}` |
| `ai.usage` | `days?` (default 30) | `[{day, provider, feature, inputTokens, outputTokens, requests}]` |

Notifications, added to the routing table of desktop-client-plan 3.1:

| Notification | Filter | Backlog | Dedup |
|---|---|---|---|
| `ai.delta {streamId, text}` | the connection that sent `streamId` | no | in order; the final reply has the full text |
| `ai.triage.progress {jobId, done, total}` | the connection that called `ai.triage.run` | no | `jobId` |

Errors: `-32602` for params that don't deserialize; an `AiError` is engine code `-32011` "AI
request failed" with the serialized error as `data`. All `ai.*` methods are slow (T21) and run
on stage 5's worker pool.

**Relay policy** (`crates/ms-ipc/src/policy.rs`): every `ai.*` method is `Open` except that a
relayed connection may never pass `config`, only `provider`. Otherwise a remote device could
make the engine send requests, and mail, to any URL, including hosts on the user's LAN.
Both rules go in `ms_ipc::policy::relayed_request_allowed` (the method classes and the
`config` refusal), which the gateway and the engine both enforce, not only in the engine's
socket code. Until then `ai.*` are unknown names, so `LocalOnly` (admin devices only).
`apps/gateway/tests/notifications.rs` (tern-web, main b9f6b53e5) is switched to assert the
`config` refusal once `ai.*` are `Open`. The gateway already routes `ai.delta` and
`ai.triage.progress` only to the WebSocket whose engine connection started the request, in
order, and closing a tab closes that connection, which cancels the provider request (A7).

**Features and capabilities.** `hello.features` gains `ai`. Stage 5's `capabilities` gains
`ai: {summary, reply, triage}`, each `{available, provider?, local?, reason?}`, with the
reasons of integrations-plan section 1 ("Set up a model in Preferences > AI", "Turn on AI
for this account", "Waiting for engine support").

## 4. Triage

- **Trigger.** An engine thread (`thread::AI`, spawned through `WorkerContext::spawn` per
  account) takes `mail.new` events for opted-in accounts into a bounded queue (1000), and
  `ai.triage.run` jobs. One request in flight per provider; a `rate-limited` or
  `quota-exceeded` error pauses the queue until `retryAfter` (or 15 minutes) and sets the
  capability reason. Default limit 200 threads per account per hour.
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
  today's read API without stage 7's `query`.
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
