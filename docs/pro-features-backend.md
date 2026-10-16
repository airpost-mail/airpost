# Pro features and the backend

The desktop has no plans, quotas or upgrade prompts, and no Pro plugin needs a Mailspring ID to load. The features below that need a server still show in the UI. Without an identity they stay inert, because `makeRequest` (`apps/desktop/app/src/flux/mailspring-api-request.ts`) refuses to send a request when no identity is set, so they make no calls to `*.getmailspring.com`. Users who signed in to a Mailspring ID before this change keep their identity, and these features still reach Mailspring's servers for them. Desktop paths below are relative to `apps/desktop/app/`. `docs/backend-plan.md` has the full treatment of each one.

| Feature | Works locally today | Mailspring endpoint it used | What `apps/backend` must provide |
|---|---|---|---|
| Snooze | Yes | `/metadata`, `/deltas` (sync only) | Metadata sync for multi-device |
| Send later | Yes | none | Nothing (drafts are local only) |
| Follow-up reminders | Yes | `/metadata`, `/deltas` (sync only) | Metadata sync for multi-device |
| Templates | Yes | none | Nothing |
| Open tracking | No | `link.getmailspring.com/o/<token>.png` | Public pixel host that writes opens into message metadata |
| Link tracking | No | `link.getmailspring.com/link/<headerMessageId>/<n>?redirect=` | Public redirector that writes clicks into message metadata |
| Activity dashboard | Partly | `/api/share-static-page` (share button) | Static page hosting |
| Thread sharing | No | `/api/save-public-asset`, viewer at `shared.getmailspring.com` | Asset upload, public storage, viewer page |
| Translation | No | `/api/translate` | Nothing; point the client at a LibreTranslate URL |
| Contact and company profiles | No | `/api/info-for-email-v2/<email>` | Nothing; no data source, keep it inert or drop it |
| Grammar check | No | `/api/grammar/check` | Nothing; point the client at a LanguageTool URL |
| Signature photo upload | No | `/api/save-public-asset` | Same storage as thread sharing |
| Feature usage events | n/a | `/api/feature_usage_event` | Nothing; the desktop no longer queues `SendFeatureUsageEventTask` |

- Snooze and reminders fire from `metadata-expiration` deltas, which the engine emits from its local database. The `LocalOnly` metadata backend (`crates/ms-net/src/metadata.rs`) is enough for a single device.
- Send later fires from the draft's own metadata on the same device.
- Open and link tracking: the composer leaves outgoing mail untouched when no identity is set, and the toolbar toggle is disabled with a tooltip (`internal_packages/{open,link}-tracking/lib/*-constants.ts`). Mailspring's tracking URLs can never reach a Tern server, so the backend needs its own URL format.
- Activity: the dashboard reads local tracking metadata. Only the share button needs a server.
- Thread sharing: the toggle shows an error dialog when no identity is set. Share URLs are hard-coded to `shared.getmailspring.com` in `internal_packages/thread-sharing/lib/main.tsx`.
- Translation: manual translation shows an error, and automatic translation fails silently. The client needs a configurable translation URL, with the adapter in `internal_packages/translation/lib/service.ts`.
- Contact and company profiles: the sidebar shows the local contact and message history, with no enrichment.
- Grammar check: the store skips checks when no identity is set (`internal_packages/composer-grammar-check/lib/grammar-check-store.ts`). The client needs a configurable LanguageTool URL.
- Signature photo upload: the upload option is hidden when no identity is set. The company-logo option still builds `logo.getmailspring.com` URLs (`internal_packages/composer-signature/lib/constants.ts`), which predates this change.
