# Integrations: capabilities, providers and the integrations crate

Draft, 2026-10-16; the design in sections 0-2 was agreed with the user on 2026-10-15/16.
Pro features that Mailspring served from its own servers either run locally, call a
provider the user configures, or wait for a Tern server. Provider logic lives in the engine
(`crates/ms-integrations`); Electron stays thin. Desktop paths are relative to
`apps/desktop/`.

## 0. Decisions

| # | Decision |
|---|---|
| I1 | One capability store in the desktop answers "is feature X available, and from where". Three sources: **local** (always), **integration** (when the user configured a provider), **server** (only with a Tern server; disabled for now, labelled "Needs a Tern server"). |
| I2 | Providers: LibreTranslate or DeepL for translation, LanguageTool for grammar, People Data Labs for contact and company enrichment. One translation provider at a time. |
| I3 | Keys live in the desktop keyset (`KeyManager`), never in `config.json` or logs. Until the engine owns credentials, the desktop sends the provider config, key included, with each request. |
| I4 | Provider HTTP, response mapping, errors and caching are in `crates/ms-integrations`, used by the engine. No provider calls from Electron. |
| I5 | Transport: `integrations.*` JSON-RPC methods on the engine socket, after phase A (engine-protocol-plan 1.2). No stdin shim; the crate is written and tested transport-free first. |
| I6 | Privacy: logs carry provider, host, status and timing only. Keys, usernames, message text and email addresses are never logged and never part of a cache key in clear. |

## 1. Capability store

`src/flux/stores/capability-store.ts`, exported from `mailspring-exports`:

```ts
type Source = 'local' | 'integration' | 'server';
CapabilityStore.get(feature): { available: boolean; source: Source; reason?: string; provider? }
CapabilityStore.isAvailable(feature): boolean
CapabilityStore.unavailableMessage(feature): string | null  // "Open tracking: Needs a Tern server"
useCapability(feature)                                      // React hook, re-renders on change
```

| Feature | Source | Available when |
|---|---|---|
| snooze, send-later, reminders, templates, insights | local | always |
| translation | integration | a translation provider is enabled and complete, and the engine transport is up |
| grammar-check | integration | LanguageTool is enabled (premium: username and key both set), and the transport is up |
| contact-enrichment | integration | People Data Labs is enabled with a key, and the transport is up |
| read-receipts, link-tracking, thread-sharing, activity-sharing, metadata-sync | server | never yet; `reason: "Needs a Tern server"` |

Reasons for an integration feature, first match wins: "Set up a provider in Preferences >
Integrations" (off or no provider), "Add the <provider> URL …", "Add your <provider> API key
…", "Add your <provider> username …", then "Waiting for engine support" while the client's
transport is the stub. The store listens to the `core.integrations` config, to `credentials`
(KeyManager rewrites that config key on every keyset change, in any window) and to transport
changes, and triggers on each. It only records whether each enabled provider's key exists, and
reads the keyset only when an integration is turned on.

## 2. Preferences > Integrations

A new tab in `features/preferences` (`tabs/preferences-integrations.tsx`). One
section per provider, each with an enable toggle, a Test button (`integrations.test`, shows
the host and, for DeepL, the characters used) and a one-line privacy note:

| Section | Fields | Privacy note |
|---|---|---|
| Translation | LibreTranslate: URL, optional key; or DeepL: key | "Message text you translate is sent to <host>." |
| Grammar | LanguageTool: URL (default `https://api.languagetool.org`), optional premium username and API key | "Drafts are sent to <host> while you type." |
| Contact info | People Data Labs: key | "Email addresses of people you view are sent to People Data Labs." |

Config (`core.integrations.<kind>`, kinds `translation`, `grammar`, `enrichment`) holds
`{provider, enabled, url?, username?}`; the LanguageTool premium username is there too.
API keys are only in the keyset (`KeyManager`) as `integration-<provider>-key`, never shown
again once saved (the field offers Forget Key). A typed key is saved with Save Key or Enter,
not on blur, so clicking Test doesn't store an untested key. Turning a provider off keeps its
key; Remove clears the section's config and deletes the keys of all its providers. Test sends the form's values, a typed key
over the saved one, as `integrations.test`; until the engine wiring lands it shows "Waiting
for engine support." without reading the keyset.

## 3. The crate (`crates/ms-integrations`)

Blocking, `Send + Sync`, one `Integrations` per engine process. Depends on reqwest
(blocking, rustls on ring), serde, sha2, thiserror and tracing; not on ms-net, whose
`SyncError` mapping and store and provider dependencies it doesn't need.

```rust
let ix = Integrations::new()?;                       // or with_options(Options { … })
ix.translate(&TranslationProvider, &TranslateRequest) -> Result<Translation>
ix.translation_languages(&TranslationProvider)      -> Result<Vec<Language>>
ix.detect_language(&TranslationProvider, text)      -> Result<Vec<DetectedLanguage>>  // LibreTranslate
ix.check_grammar(&LanguageToolConfig, &GrammarRequest) -> Result<GrammarCheck>
ix.grammar_languages(&LanguageToolConfig)           -> Result<Vec<Language>>
ix.enrich_person(&PeopleDataLabsConfig, email)      -> Result<Option<Person>>
ix.enrich_company(&PeopleDataLabsConfig, domain)    -> Result<Option<Company>>
ix.enrich_contact(&PeopleDataLabsConfig, email)     -> Result<ContactProfile>
ix.test(&ProviderConfig)                            -> Result<TestReport>
ix.clear_cache()
```

Configs deserialize from the request JSON (`{"provider":"deepl","apiKey":"…"}`); keys are
`Secret`, which has no `Serialize` and prints `Secret(***)`, and config `Debug` replaces URL
userinfo with `***`. Optional URL settings (DeepL, LanguageTool, PDL) fall back to the
default when absent, `null` or blank.

Language codes are the desktop's (`AllLanguages`): `no` goes out as `nb` (both providers
only have Bokmål), and `nb` and `zh-Hans` (LibreTranslate's name for `zh`, DeepL's
Simplified Chinese) come back as `no` and `zh` in `detectedLanguage` and language lists.
Region variants without a desktop key (`pt-br`, `zh-hant`) are passed through lowercase.

- **LibreTranslate**: `POST /translate` (`q`, `source` or `auto`, `target`, `format`,
  `api_key` in the body), `/detect`, `GET /languages`. A keyless request to a key-requiring
  server (400 "… to get an API key") is `auth`; the flood ban (403) is `rate-limited`.
- **DeepL**: `POST /v2/translate` with `Authorization: DeepL-Auth-Key`; keys ending `:fx`
  go to `api-free.deepl.com`, others to `api.deepl.com`. HTML uses `tag_handling=html`.
  `EN`/`PT` targets become `EN-US`/`PT-BR`. 456 is `quota-exceeded`. No detection
  (`unsupported`).
- **LanguageTool**: `POST /v2/check` (form: `text`, `language` or `auto`,
  `disabledRules`, `preferredVariants`, `username` + `apiKey`). Premium credentials with the
  default URL go to `api.languagetoolplus.com`; a username without a key (or the reverse)
  is `not-configured` rather than a silent free-tier check. Misspellings are dropped unless asked for;
  the composer's spellchecker marks them.
- **People Data Labs**: `GET /v5/person/enrich?email=` (`min_likelihood=6`) and
  `/v5/company/enrich?website=`, `X-Api-Key`. A 404 with `error.type` `not_found` is "no
  match" (`None`, not billed); any other 404 is `bad-request` and isn't cached. 402 is
  `quota-exceeded`. `enrich_contact` looks up the company from the person's employer
  website, else from the address's domain unless it is webmail or ISP mail (a domain list
  plus per-country families such as `yahoo.*`, `hotmail.*`, `gmx.*`).

Output shapes are what the plugins already consume: `Translation` is LibreTranslate's
`{translatedText, detectedLanguage?}`; `GrammarError` is
`composer-grammar-check/lib/grammar-check-service.ts`'s; `Person`, `Company` and
`ContactProfile {person?, company?}` are the sidebar's `IPerson` / `ICompany` state.

`test()` is the cheapest call that proves URL and credentials: LibreTranslate `/detect` of
one word (`/languages` needs no key), DeepL `/v2/usage` (bills nothing), LanguageTool one
short check (`/v2/languages` ignores credentials), PDL `/v5/autocomplete` (free for active
keys; enrichment bills per match; anything but 200 fails).

**Errors.** One `IntegrationError`, serialized as `{key, message, retryable, status?,
retryAfter?}`:

| key | retryable | when |
|---|---|---|
| `not-configured` | no | URL or key blank |
| `auth` | no | 401/403, LibreTranslate's missing-key 400 |
| `rate-limited` | yes | 429 (`retryAfter` from `Retry-After` seconds) |
| `quota-exceeded` | no | DeepL 456, PDL 402 |
| `offline` | yes | DNS, connect, TLS, timeout (10 s connect, 30 s total) |
| `bad-request` | no | other 4xx, malformed URL setting, unexpected redirect |
| `provider-error` | yes | 5xx, unreadable 2xx body |
| `unsupported` | no | detection on DeepL |

**Cache.** In memory, per kind, TTL plus an entry bound (oldest insert evicted after expired
ones), keyed by SHA-256 of provider, endpoint, key and the normalised request. Translations
24 h / 1000 entries; people and companies 7 days, no-match 24 h, 500 entries each. Errors
and grammar checks are not cached.

## 4. Socket methods (after phase A)

Not account-scoped (no `accountId`). In phase A the desktop calls them on any connected
engine; in phase C on the one engine. `config` is the provider config of section 3, key
included (I3).

| Method | Params | Result |
|---|---|---|
| `integrations.translate` | `config`, `text`, `source?`, `target`, `format?` (`text`/`html`) | `Translation` |
| `integrations.detect` | `config`, `text` | `[DetectedLanguage]` |
| `integrations.languages` | `config` (any translation provider or LanguageTool) | `[Language]` |
| `integrations.grammar` | `config`, `text`, `language?`, `disabledRules?`, `preferredVariants?`, `includeMisspellings?` | `GrammarCheck` |
| `integrations.enrich` | `config`, `email` | `ContactProfile` |
| `integrations.test` | `config` | `TestReport {host, usage?}` |

Errors: invalid params (`-32602`) for JSON that doesn't deserialize; any `IntegrationError`
is engine code `-32010` "Integration failed" with the serialized error as `data`. The
dispatcher runs these on a worker pool, not the connection's reader thread, since a call can
take up to 30 s.

## 5. Desktop (built, without the engine wiring)

- `src/flux/stores/capability-store.ts`: `CapabilityStore` and `useCapability`, exported from
  `mailspring-exports` (section 1).
- `src/integrations.ts`, exported as `Integrations`: provider metadata, `readSettings` /
  `writeSettings`, `buildProviderConfig`, and `request(method, params, {kind?, config?})`,
  which adds `config` (saved settings plus the key read from the keyset per request, or an
  explicit config for Test) to the params of section 4. A disabled integration is
  `not-configured` without a call. Errors become `IntegrationError {key, message, retryable,
  status?, retryAfter?}`, from the bare serialized error or the socket error's `data`;
  `errorMessage(err, providerName)` gives the short user message per key. The desktop adds
  `engine-unavailable` and `unknown` to the crate's keys.
- Transport: `Integrations.setTransport({available, send(method, params)})`. The default is
  `UnavailableTransport` (`available: false`, every call fails with `engine-unavailable`); while
  it is in place the store reports no integration feature as available. Since stage 5 every
  window sets one over its engine connection (`src/engine-api.ts`), available while the engine
  lists the `integrations` feature; `-32010` becomes the error in `data`, `-32012` (busy)
  `rate-limited`, a dropped connection `engine-unavailable`.
- `preferences/lib/tabs/preferences-integrations.tsx`: the tab of section 2, order 3.
- `translation`: `requestTranslation` calls `integrations.translate` with `format: "html"` and
  returns `translatedText`; `isTranslationAvailable()` and the composer button's tooltip read
  the store; the message header starts detection when the store changes and its privacy note
  names the LibreTranslate host or DeepL. The snippet LRU and CLD detection stay.
- `composer-grammar-check`: `LanguageToolBackend.check` calls `integrations.grammar`
  (`language` omitted for auto, `disabledRules` from config) and returns `matches` as is;
  `isAvailable()` and the toggle's tooltip read the store, and `GrammarCheckStore` triggers on
  store changes.
- `participant-profile`: `find()` returns the localStorage profile, else calls
  `integrations.enrich` when `contact-enrichment` is available and stores a match (not a
  no-match or an error, so those are retried on a later view).
- Server features (open and link tracking, thread sharing, the activity share button) take
  their disabled tooltip from `unavailableMessage`; tracking also still needs `PLUGIN_URL`.

## 6. Open

- The Test button's host and usage output is untested against a real provider through
  `integrations.test` (the engine transport landed in stage 5, `src/engine-api.ts`).
- Moving keys from the desktop keyset to the engine (with the engine owning credentials
  generally), after which `config` in section 4 drops the key.
- DeepL marks `/v2/languages` deprecated for `/v3/languages` (no sunset date yet).
- A LanguageTool premium quota answer is undocumented; today it maps by status.
