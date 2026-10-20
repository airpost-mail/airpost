# Pro features and the backend

The desktop has no plans, quotas or upgrade prompts, and no Mailspring ID: there is no sign-in, no identity store and no client for Mailspring's API. A Mailspring ID carried over in config.json (`identity`) and its keyset token are ignored and left in place, because the profile may still be shared with Mailspring.app. Sync engines are spawned with a `null` identity, so the engine uses its `LocalOnly` metadata backend. The features below that need a server still show in the UI, disabled with a tooltip, and make no network calls until `apps/backend` provides them. Desktop paths below are relative to `apps/desktop/`. `docs/backend-plan.md` has the full treatment of each one. Connecting a server (plan 3.4, 4.2, B1-C) is a Tern login against a user-configured URL: one device token in the keyset, passed only to the Rust engine as a Tern server object on its identity line; the C++ engine keeps getting `null`.

| Feature | Works locally today | Mailspring endpoint it used | What `apps/backend` must provide |
|---|---|---|---|
| Snooze | Yes | `/metadata`, `/deltas` (sync only) | Metadata sync for multi-device |
| Send later | Yes | none | Nothing (drafts are local only) |
| Follow-up reminders | Yes | `/metadata`, `/deltas` (sync only) | Metadata sync for multi-device |
| Templates | Yes | none | Nothing |
| Open tracking | No | `link.getmailspring.com/o/<token>.png` | Public pixel host that writes opens into message metadata (B2-T); `publicURL` and `trackingNS` from `GET /tern/v1/me` |
| Link tracking | No | `link.getmailspring.com/link/<headerMessageId>/<n>?redirect=` | Public redirector that writes clicks into message metadata |
| Activity dashboard | Partly | `/api/share-static-page` (share button) | Static page hosting, `POST /tern/v1/pages` (B2-S) |
| Thread sharing | No | `/api/save-public-asset`, viewer at `shared.getmailspring.com` | Asset upload (`POST /tern/v1/assets`), public storage, viewer page (B2-S) |
| Translation | Not yet; a configured provider, via the engine | `/api/translate` | Nothing; LibreTranslate or DeepL via the engine |
| Contact and company profiles | Not yet; a configured provider, via the engine | `/api/info-for-email-v2/<email>` | Nothing; People Data Labs via the engine |
| Grammar check | Not yet; a configured provider, via the engine | `/api/grammar/check` | Nothing; LanguageTool via the engine |
| Signature photo upload | No | `/api/save-public-asset` | Same storage as thread sharing (B2-S) |
| Feature usage events | n/a | `/api/feature_usage_event` | Nothing; the desktop no longer queues `SendFeatureUsageEventTask` |

- Snooze and reminders fire from `metadata-expiration` deltas, which the engine emits from its local database. The `LocalOnly` metadata backend (`crates/ms-net/src/metadata.rs`) is enough for a single device.
- Send later fires from the draft's own metadata on the same device.
- Open and link tracking: the engine adds the pixel and rewrites the links when it sends a draft with tracking on, in the Tern format (backend plan 3.6), from the connected server's `publicURL` and `trackingNS`; the desktop's composer extensions are gone. The capability store reports `read-receipts` and `link-tracking` from the engine's `capabilities.server`, so the toolbar toggles stay disabled with the store's tooltip until a server that offers tracking is connected. The server's `/t/o` and `/t/l` endpoints don't exist yet (apps/backend).
- Activity: the dashboard reads local tracking metadata. Only the share button needs a server; it is disabled with the capability store's tooltip (`plugins/activity/lib/dashboard/share-button.tsx`).
- Thread sharing: the toolbar button is disabled with the capability store's tooltip, and the Share this thread menu item shows the same message (`plugins/thread-sharing/lib/thread-sharing-button.tsx`). The upload, the share popover and the viewer URL need writing against the backend.
- Translation: a provider (LibreTranslate or DeepL) set up in Preferences > Integrations, called by the engine; see `docs/integrations-plan.md`. Until the engine wiring lands the composer button is disabled with the capability store's tooltip and messages get no translation banner.
- Contact and company profiles: People Data Labs set up in Preferences > Integrations, called by the engine (`docs/integrations-plan.md`). Without it the sidebar shows the local contact and message history, plus any profile cached in localStorage (`plugins/participant-profile/lib/participant-profile-data-source.ts`).
- Grammar check: LanguageTool set up in Preferences > Integrations, called by the engine (`docs/integrations-plan.md`). Until then `LanguageToolBackend.isAvailable()` follows the capability store, the store skips checks and the toolbar toggle is disabled with the store's tooltip.
- Signature photo upload: the picker has no upload, only a URL for a custom image (`features/composer-signature/lib/signature-photo-picker.tsx`). The company-logo option still builds `logo.getmailspring.com` URLs (`features/composer-signature/lib/constants.ts`).
