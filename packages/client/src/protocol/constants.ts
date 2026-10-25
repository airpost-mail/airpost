// SPDX-License-Identifier: GPL-3.0-only
// Generated from crates/tern-ipc by packages/client/scripts/generate.mjs. Do not edit: change
// the Rust types, then run `npm run generate` in packages/client.

/** Method and notification names. (tern-ipc `method`) */
export const Method = {
  Hello: 'hello',
  Subscribe: 'subscribe',
  TaskQueue: 'task.queue',
  TaskCancel: 'task.cancel',
  SyncWake: 'sync.wake',
  BodiesFetch: 'bodies.fetch',
  CalendarSync: 'calendar.sync',
  EngineStatus: 'engine.status',
  FoldersList: 'folders.list',
  LabelsList: 'labels.list',
  ThreadsList: 'threads.list',
  ThreadsGet: 'threads.get',
  MessagesGet: 'messages.get',
  Search: 'search',
  ContactsSearch: 'contacts.search',
  TasksGet: 'tasks.get',
  /** Every `ThreadCounts` row of the accounts (`feature::COUNTS`). */
  CountsGet: 'counts.get',
  /** Stored tasks by account, status, class or draft (`feature::TASKS_LIST`). */
  TasksList: 'tasks.list',
  /**
   * Messages or threads carrying a plugin's metadata
   * (`feature::METADATA_LIST`).
   */
  MetadataList: 'metadata.list',
  /** A ModelQuery over the stored models (`feature::QUERY`). */
  Query: 'query',
  /**
   * A File by id, downloaded first when missing (`feature::FILES`),
   * params `FilesGetParams`. Local clients only; answered out of
   * order while it waits for the download.
   */
  FilesGet: 'files.get',
  /** A chunk of a downloaded File, params `FilesReadParams`. */
  FilesRead: 'files.read',
  /** Stores bytes as a new File for a draft, params `FilesPutParams`. */
  FilesPut: 'files.put',
  /**
   * Moves threads to Snoozed with the metadata that brings them back
   * (`feature::SNOOZE`), params
   * `ThreadsSnoozeParams`.
   */
  ThreadsSnooze: 'threads.snooze',
  /** Ends a snooze now, params `ThreadsUnsnoozeParams`. */
  ThreadsUnsnooze: 'threads.unsnooze',
  /**
   * Unsubscribes from the list a message came from
   * (`feature::UNSUBSCRIBE`), params
   * `MessagesUnsubscribeParams`. Socket only; answered
   * out of order while a one-click POST runs.
   */
  MessagesUnsubscribe: 'messages.unsubscribe',
  /**
   * Builds a draft as the desktop does and saves it
   * (`feature::DRAFT_PIPELINE`), params
   * `DraftCreateParams`; answers with its Message JSON.
   */
  DraftCreate: 'draft.create',
  /**
   * Sends a draft now or after an undo window, params
   * `DraftSendParams`.
   */
  DraftSend: 'draft.send',
  /**
   * Cancels a send waiting out its undo window, params
   * `DraftCancelSendParams`.
   */
  DraftCancelSend: 'draft.cancelSend',
  /** The checks before a send, params `DraftValidateParams`. */
  DraftValidate: 'draft.validate',
  /**
   * An edit to a stored draft (`feature::DRAFT_EDITING`),
   * params `DraftUpdateParams`; answers with its Message JSON.
   */
  DraftUpdate: 'draft.update',
  /** Deletes a draft, params `DraftDeleteParams`. */
  DraftDelete: 'draft.delete',
  /**
   * The templates (`feature::TEMPLATES`), result
   * `TemplatesListResult`.
   */
  TemplatesList: 'templates.list',
  /**
   * One template with its body, params `TemplateNameParams`,
   * result `TemplateResult`.
   */
  TemplatesGet: 'templates.get',
  /**
   * Saves a template, params `TemplateSaveParams`, result
   * `TemplateResult`. Local clients only.
   */
  TemplatesSave: 'templates.save',
  /**
   * Deletes a template's file, params `TemplateNameParams`.
   * Local clients only.
   */
  TemplatesDelete: 'templates.delete',
  /**
   * Occurrences of stored events or of an ICS
   * (`feature::CALENDAR_EDITING`), params
   * `EventsExpandParams`.
   */
  EventsExpand: 'events.expand',
  /** A new event, params `EventsCreateParams`. */
  EventsCreate: 'events.create',
  /**
   * An edit to an event or some of its occurrences, params
   * `EventsUpdateParams`.
   */
  EventsUpdate: 'events.update',
  /**
   * An answer to an invitation or a counter-proposal, params
   * `EventsRsvpParams`.
   */
  EventsRsvp: 'events.rsvp',
  /**
   * A contact's details (`feature::CONTACT_EDITING`),
   * params `ContactsGetParams`.
   */
  ContactsGet: 'contacts.get',
  /** A new contact, params `ContactsCreateParams`. */
  ContactsCreate: 'contacts.create',
  /** An edit to a contact, params `ContactsUpdateParams`. */
  ContactsUpdate: 'contacts.update',
  /** Contacts from a .vcf file, params `ContactsImportParams`. */
  ContactsImport: 'contacts.import',
  /** Contacts as a .vcf file, params `ContactsExportParams`. */
  ContactsExport: 'contacts.export',
  /** Every account the engine holds, with its state. */
  AccountList: 'account.list',
  /**
   * `serve` mode: register an account, store its secrets and start syncing it
   * (`AccountAddParams`). On the parent's stdin, and on the
   * socket of an engine with `feature::ACCOUNT_SETUP`
   * (local-only class, `policy`).
   */
  AccountAdd: 'account.add',
  /**
   * `serve` mode: stop an account, unregister it and delete its secrets, with `revoke`
   * also its OAuth grant and with `deleteData` its rows
   * (`AccountRemoveParams`). Without `deleteData` the rows
   * stay. Where `account.add` is taken.
   */
  AccountRemove: 'account.remove',
  /**
   * IMAP and SMTP settings for an address (`AccountDiscoverParams`).
   * Open to relayed connections; answered out of order (MX and autoconfig lookups).
   */
  AccountDiscover: 'account.discover',
  /**
   * Starts an OAuth sign-in: the engine keeps the PKCE verifier, `state` and client
   * secret (`OAuthBeginParams`).
   */
  AccountOAuthBegin: 'account.oauth.begin',
  /**
   * Ends it with the redirect's `code` and `state`: exchange, profile, discovery, test,
   * then the account is stored and started (`OAuthCompleteParams`).
   * Answered out of order.
   */
  AccountOAuthComplete: 'account.oauth.complete',
  /** New credentials for a registered account (`AccountSetSecretsParams`). */
  AccountSetSecrets: 'account.setSecrets',
  /**
   * Changes an account's JSON (`AccountUpdateParams`).
   * Relayed devices without the admin scope may change only
   * `ACCOUNT_DISPLAY_FIELDS`.
   */
  AccountUpdate: 'account.update',
  /** The order of `account.list` (`AccountReorderParams`). */
  AccountReorder: 'account.reorder',
  /**
   * Stores a secret that isn't an account's (`SecretsSetParams`).
   * There is no `secrets.get`.
   */
  SecretsSet: 'secrets.set',
  SecretsDelete: 'secrets.delete',
  /** Which of the named secrets are stored (`SecretsHasParams`). */
  SecretsHas: 'secrets.has',
  /** The Tern server the accounts sync metadata with (`ServerSetParams`). */
  ServerSet: 'server.set',
  /** Forgets it and its token; accounts sync metadata locally. */
  ServerClear: 'server.clear',
  /**
   * `serve` mode, the parent's stdin only (the socket too on a `--detached` engine): stop
   * every account, remove the socket files and exit 0.
   */
  EngineShutdown: 'engine.shutdown',
  /**
   * `serve` mode, the parent's stdin only: register accounts and store secrets a client
   * held until now, params
   * `CredentialsImportParams`.
   */
  CredentialsImport: 'credentials.import',
  /**
   * Notification: an account's registered JSON changed, `params` =
   * `AccountChangedParams`. Filtered by account, never held
   * for a later subscriber: a client calls `account.list` after `subscribe`.
   */
  AccountChanged: 'account.changed',
  /**
   * `serve` mode: check an account's IMAP and SMTP settings, as `--mode test` does
   * (`AccountTestParams`). The result is that mode's JSON
   * object, with any rotated refresh token in its `account` on stdin only. Where
   * `account.add` is taken; answered out of order.
   */
  AccountTest: 'account.test',
  /**
   * `serve` mode: stop the account if the engine holds it, delete its rows from the
   * database (`--mode reset`) and start it again from the registry to resync (a client
   * that adds it again gets ACCOUNT_EXISTS). Where `account.add` is taken; answered out of
   * order.
   */
  AccountReset: 'account.reset',
  /**
   * `serve` mode, the parent's stdin only: stop and forget every account, delete the
   * database and create an empty one. The client adds the accounts again.
   */
  EngineResetDatabase: 'engine.resetDatabase',
  /**
   * `serve` mode, the parent's stdin only: verbose protocol logging until a time, for
   * every account; running accounts restart to pick it up.
   */
  EngineSetLogLevel: 'engine.setLogLevel',
  /** The end of an account's log, or of the engine's own log. */
  EngineLogsTail: 'engine.logs.tail',
  /**
   * The stored engine-owned settings (`feature::SETTINGS`),
   * params `SettingsGetParams`, result
   * `SettingsValues`.
   */
  SettingsGet: 'settings.get',
  /** Stores settings, params `SettingsValues`, result `{}`. */
  SettingsSet: 'settings.set',
  /**
   * `serve` mode, the parent's stdin only, debug builds only: stop one account as a
   * fatal error would (`test-crash` on stdin in `sync` mode).
   */
  TestCrash: 'test.crash',
  /** As `TEST_CRASH`, with a panic on the account's `main` thread. */
  TestPanic: 'test.panic',
  /**
   * Notification: an account changed state, `params` =
   * `AccountStateParams`.
   */
  AccountState: 'account.state',
  /** Notification: one flushed delta, `params` = `DeltaParams`. */
  Delta: 'delta',
  /**
   * Notification: the engine is about to close the connection, `params` =
   * `DisconnectParams`.
   */
  Disconnect: 'disconnect',
  /**
   * Notification: new mail, `params` = `MailNewParams`. Filtered by
   * account only, never held for a later subscriber (`feature::MAIL_NEW`).
   */
  MailNew: 'mail.new',
  /**
   * Notification: settings another connection (or the parent) changed, `params` =
   * `SettingsValues` with the new values, null for a cleared key.
   * Sent to every subscribed connection whatever its filter, never held for a later
   * subscriber: a client calls `settings.get` after `subscribe`.
   */
  SettingsChanged: 'settings.changed',
  /**
   * The `integrations.*` methods (`feature::INTEGRATIONS`).
   * Each carries the
   * provider's `config`, key included, until the engine owns the credentials. Local
   * clients only, answered out of order on the engine's integrations workers (T21); a
   * provider failure is `code::INTEGRATION_FAILED`.
   */
  IntegrationsTranslate: 'integrations.translate',
  IntegrationsDetect: 'integrations.detect',
  IntegrationsLanguages: 'integrations.languages',
  IntegrationsGrammar: 'integrations.grammar',
  IntegrationsEnrich: 'integrations.enrich',
  IntegrationsTest: 'integrations.test',
  /**
   * Notification: the engine's `capabilities` changed (`feature::CAPABILITIES`),
   * `params` = the new capabilities object, whole. Sent to every subscribed connection
   * whatever its filter, never held for a later subscriber: `hello` and `engine.status`
   * carry the current value.
   */
  CapabilitiesChanged: 'capabilities.changed',
  /**
   * The `ai.*` methods (`feature::AI`), params and results
   * in `crate::AiSettings`. Until the engine keeps the AI
   * settings and keys, local clients send the provider `config`, key included, and the
   * `settings` with each call; a relayed connection may send neither
   * (`policy`). Answered out of order on the engine's AI workers (T21);
   * a failed model call is `code::AI_FAILED`.
   */
  AiModels: 'ai.models',
  AiTest: 'ai.test',
  AiSummarize: 'ai.summarize',
  AiSuggestReplies: 'ai.suggestReplies',
  AiDraftReply: 'ai.draftReply',
  /** Answered at once, in order: stops the calling connection's stream `streamId`. */
  AiCancel: 'ai.cancel',
  /** Token counts per day, provider and feature since the engine started. */
  AiUsage: 'ai.usage',
  /** Inbox triage; not served yet. */
  AiTriageRun: 'ai.triage.run',
  AiTriageList: 'ai.triage.list',
  /**
   * Notification: the next piece of a streamed answer, `params` =
   * `AiDeltaParams`. Sent only to the connection that started the
   * stream, in order, without `subscribe`; the call's response comes after the last one
   * and carries the whole text.
   */
  AiDelta: 'ai.delta',
  /** Notification: triage progress for the connection that called `ai.triage.run`. */
  AiTriageProgress: 'ai.triage.progress',
  /**
   * Mail rules (`feature::RULES`): params and results in
   * `crate::RulesReprocessParams` and `crate::RulesCancelParams`. The rules themselves
   * are the `mailRules` setting.
   */
  RulesReprocess: 'rules.reprocess',
  RulesCancel: 'rules.cancel',
  /**
   * Notification: a `rules.reprocess` job's progress, to the connection that started it
   * only, without `subscribe`; never held.
   */
  RulesProgress: 'rules.progress',
  /**
   * Notification: the engine disabled a failing rule. Sent to every subscribed connection
   * whatever its filter, never held: the rule's `disabled` and `disabledReason` are saved in
   * `mailRules`, and `settings.changed` carries them as well.
   */
  RulesDisabled: 'rules.disabled',
} as const;
export type Method = (typeof Method)[keyof typeof Method];

/** What `hello.features` may list. Clients gate on these, never on versions. (tern-ipc `feature`) */
export const Feature = {
  /** `counts.get`, and `ThreadCounts` deltas whenever a category's counts change. */
  Counts: 'counts',
  /** `tasks.list`. */
  TasksList: 'tasks-list',
  /** `metadata.list`. */
  MetadataList: 'metadata-list',
  /** `query`. Its `search.grammar: true` comes with `SEARCH_GRAMMAR`. */
  Query: 'query',
  /**
   * The search grammar (`from:`, `in:`, `is:`, `has:attachment`, `before:`/`after:`,
   * `AND`/`OR`/`NOT`, parentheses) in `query.search {grammar: true, tz, now}` and in
   * `search {grammar: true, tz, now}`, and `search {accounts}`.
   */
  SearchGrammar: 'search-grammar',
  /** `hidden` on the Message JSON the read methods return (`Message.isHidden()`). */
  MessageHidden: 'message-hidden',
  /** `mail.new` notifications for subscribed connections. */
  MailNew: 'mail-new',
  /**
   * `hello.relayed`: the engine applies `policy` to relayed
   * connections. `tern-gateway` refuses to relay to an engine without it, which would
   * ignore the field.
   */
  Relayed: 'relayed',
  /** `settings.get`, `settings.set` and `settings.changed`. */
  Settings: 'settings',
  /**
   * `files.get`, `files.read`, `files.put`, and GetMessageRFC2822Task's file-id mode (no
   * `filepath`: the export becomes the File `fileId`, set by the local phase).
   */
  Files: 'files',
  /**
   * The engine acts on due `send-later`, `thread-snooze` and `send-reminders` metadata
   * itself; only `send-later` rows marked
   * `clientFinalize`, and other plugins' rows, still come as `metadata-expiration` deltas
   * for a client to act on.
   */
  ExpirationExecutor: 'expiration-executor',
  /** `threads.snooze` and `threads.unsnooze`. */
  Snooze: 'snooze',
  /** `messages.unsubscribe`. */
  Unsubscribe: 'unsubscribe',
  /**
   * `serve`: `draft.create`, `draft.send` (with `bodies` kept with the pending send),
   * `draft.cancelSend` and `draft.validate`, and the `composing.quoteTexts` setting.
   */
  DraftPipeline: 'draft-pipeline',
  /** `serve`: `draft.update`, `draft.delete`, and `attach` on `draft.create`. */
  DraftEditing: 'draft-editing',
  /**
   * `serve`: `templates.list|get|save|delete`, `template` on `draft.create`, and the
   * `template-variable` warning of `draft.validate`.
   */
  Templates: 'templates',
  /**
   * `warnings` (`MessageWarning`) on the Message JSON of
   * `messages.get` and `threads.get`, left out when there are none.
   */
  MessageWarnings: 'message-warnings',
  /**
   * `draft.send` and the expiration executor send drafts with `open-tracking` or
   * `link-tracking` on tracked, in the Tern URL format, when the connected server offers
   * tracking (`capabilities.server.readReceipts` / `linkTracking`); the executor claims due
   * rows on the server first (docs/backend-plan.md 3.6, 3.8). Clients don't transform
   * tracked drafts themselves.
   */
  Tracking: 'tracking',
  /** The `integrations.*` methods. */
  Integrations: 'integrations',
  /**
   * `capabilities` in `hello` and `engine.status`, and `capabilities.changed`
   * (`Capabilities`).
   */
  Capabilities: 'capabilities',
  /** The `ai.*` methods but triage, `ai.delta`, and `capabilities.ai`. */
  Ai: 'ai',
  /**
   * The engine owns the accounts: the registry
   * (`accounts.json`) and secret store behind `account.add` and `account.remove` (which
   * unregisters and deletes the secrets), accounts started at launch, rotated tokens
   * stored instead of sent, the restart policy (`retryAt`, `authFailed`),
   * `credentials.import`, `account.changed`, and `credentials`, `build` and `detached` in
   * `hello`.
   */
  Accounts: 'accounts',
  /**
   * Triage: the per-account triage of new inbox mail,
   * `ai.triage.run`, `ai.triage.list`, `ai.triage.progress`, `capabilities.ai.triage`, and
   * the `core.ai.*` settings keys.
   */
  AiTriage: 'ai-triage',
  /**
   * AI providers' keys in the secret store: stored providers read
   * `integration:ai-<id>`, the AI calls take `provider` without a key, and a stored
   * provider's key goes when `settings.set` removes the provider or points it at another
   * kind or server.
   */
  AiKeys: 'ai-keys',
  /**
   * Account setup on the socket: `account.discover`,
   * `account.oauth.begin|complete`, `account.add` and `account.test` with `secrets`,
   * `account.setSecrets`, `account.update`, `account.reorder`, `account.remove {revoke,
   * deleteData}`, `account.reset`, `secrets.*` and `server.*`; ids derived by the engine;
   * `integrations.*` read a missing key from the secret `integration:<provider>`.
   */
  AccountSetup: 'account-setup',
  /**
   * `serve` applies the `mailRules` setting to arriving mail itself (T12), with
   * `rules.reprocess`, `rules.cancel`, `rules.progress`, `rules.disabled` and the
   * `mailRulesAutoSince` setting. Clients that see it stop evaluating rules.
   */
  Rules: 'rules',
  /**
   * `events.expand`, `events.create`, `events.update` and `events.rsvp`, and
   * SyncbackEventTask's `restore` (an undo or redo re-sent over the copy stored now).
   */
  CalendarEditing: 'calendar-editing',
  /**
   * `contacts.get`, `contacts.create`, `contacts.update`, `contacts.import` and
   * `contacts.export`.
   */
  ContactEditing: 'contact-editing',
} as const;
export type Feature = (typeof Feature)[keyof typeof Feature];

/** Error codes: the standard JSON-RPC codes, then the engine's from -32000 down. (tern-ipc `code`) */
export const ErrorCode = {
  /** The line is not JSON. The response's `id` is null. */
  ParseError: -32700,
  /** JSON, but not a request object. */
  InvalidRequest: -32600,
  MethodNotFound: -32601,
  InvalidParams: -32602,
  InternalError: -32603,
  /**
   * `hello` named a protocol version the engine does not speak; `data.supported` lists
   * the ones it does.
   */
  UnsupportedProtocol: -32000,
  /** The `accountId` is not one the engine holds. */
  UnknownAccount: -32001,
  /** The task's local phase failed; `data` is the `SyncError` JSON. */
  TaskRejected: -32002,
  /** A request other than `hello` came before a successful `hello`. */
  HelloRequired: -32003,
  /** A read named a folder, label, thread or message the account does not have. */
  NotFound: -32004,
  /**
   * The account is known but not running (it stopped on an error, or is being
   * removed); `data` is `{state, error}`.
   */
  AccountStopped: -32005,
  /** `account.add` named an account that is already starting or running. */
  AccountExists: -32006,
  /**
   * The method is only accepted on the parent's stdin (`credentials.import`,
   * `engine.shutdown`, ...), or only on the socket (`subscribe`).
   */
  WrongChannel: -32007,
  /**
   * A relayed connection asked for a method or task its device's scopes don't allow
   * (`policy`).
   */
  ForbiddenRelayed: -32008,
  /** The engine already has as many socket connections as it accepts. */
  TooManyConnections: -32009,
  /**
   * "Integration failed": a provider call failed; `data` is the serialized
   * `IntegrationError` `{key, message, retryable, status?, retryAfter?}`.
   */
  IntegrationFailed: -32010,
  /**
   * "AI request failed": a model call failed or the account or feature is off; `data`
   * is the serialized `AiError` `{key, message, retryable, status?, retryAfter?}`.
   */
  AiFailed: -32011,
  /**
   * The engine's workers for this kind of call are all busy and their queue is full; the
   * client may try again later.
   */
  EngineBusy: -32012,
  /**
   * "Account test failed": `account.add {test}`, `account.setSecrets {test}` or
   * `account.oauth.complete` found the settings or credentials don't work; `data` is the
   * `account.test` result (`error`, `error_service`, `log`, ...).
   */
  AccountTestFailed: -32013,
  /**
   * "OAuth sign-in failed": the code exchange or the profile request failed, or the
   * account has no mailbox; `data` is `{key, status?, providerError?}`.
   */
  OAuthFailed: -32014,
} as const;
export type ErrorCode = (typeof ErrorCode)[keyof typeof ErrorCode];

/** An account's `state` in `hello`, `engine.status`, `account.list` and `account.state`. (tern-ipc `account_state`) */
export const AccountState = {
  /** Added; its `main` thread has not opened the database yet. */
  Starting: 'starting',
  Running: 'running',
  /**
   * Stopped by a failure (`error` says which). The engine does not restart it; the
   * client may `account.add` it again.
   */
  Stopped: 'stopped',
  /** `account.remove` finished; only sent as an `account.state` notification. */
  Removed: 'removed',
} as const;
export type AccountState = (typeof AccountState)[keyof typeof AccountState];

/** Why a capability is unavailable (`Capability.reason`). (tern-ipc `capability_reason`) */
export const CapabilityReason = {
  /** No provider is chosen, or it is turned off. */
  NotSetUp: 'not-set-up',
  /** The provider needs a URL (LibreTranslate) and has none. */
  MissingUrl: 'missing-url',
  /** Only a Tern server offers it, and none is connected. */
  NeedsTernServer: 'needs-tern-server',
  /** A Tern server is connected but doesn't offer it (its `features` leave it off). */
  NotOnServer: 'not-on-server',
  /**
   * AI: the provider needs an API key and the secret store holds none under
   * `integration:ai-<id>` (Ollama needs none).
   */
  NeedsEngineKey: 'needs-engine-key',
  /** AI triage: the provider is rate limiting; triage waits and resumes by itself. */
  RateLimited: 'rate-limited',
  /** AI triage: the provider's quota or the daily token cap is used up. */
  QuotaExceeded: 'quota-exceeded',
} as const;
export type CapabilityReason = (typeof CapabilityReason)[keyof typeof CapabilityReason];

/** The kinds in `Capabilities.integrations`. (tern-ipc `integration_kind`) */
export const IntegrationKind = {
  Translation: 'translation',
  Grammar: 'grammar',
  Enrichment: 'enrichment',
} as const;
export type IntegrationKind = (typeof IntegrationKind)[keyof typeof IntegrationKind];

/** The features in `Capabilities.server`. (tern-ipc `server_feature`) */
export const ServerFeature = {
  ReadReceipts: 'readReceipts',
  LinkTracking: 'linkTracking',
  ThreadSharing: 'threadSharing',
  ActivitySharing: 'activitySharing',
  MetadataSync: 'metadataSync',
} as const;
export type ServerFeature = (typeof ServerFeature)[keyof typeof ServerFeature];

/** The features in `Capabilities.ai`. (tern-ipc `ai_feature`) */
export const AiFeature = {
  Summary: 'summary',
  Reply: 'reply',
  Triage: 'triage',
} as const;
export type AiFeature = (typeof AiFeature)[keyof typeof AiFeature];

/** `messages.unsubscribe` methods. (tern-ipc `unsubscribe_method`) */
export const UnsubscribeMethod = {
  /** RFC 8058: the engine POSTs `List-Unsubscribe=One-Click` to the https URI. */
  OneClick: 'one-click',
  /** RFC 2369 `mailto:`: the engine sends the message the URI describes. */
  Mailto: 'mailto',
  /** An http(s) URI for the client to open. */
  Web: 'web',
} as const;
export type UnsubscribeMethod = (typeof UnsubscribeMethod)[keyof typeof UnsubscribeMethod];

/** `draft.create` kinds. (tern-ipc `draft_kind`) */
export const DraftKind = {
  New: 'new',
  Reply: 'reply',
  ReplyAll: 'reply-all',
  Forward: 'forward',
  Mailto: 'mailto',
  /**
   * The self-addressed message that brings a thread to the top of the inbox (snooze,
   * reminders); `threadId`, and `messageId` for the message it answers.
   */
  Resurface: 'resurface',
} as const;
export type DraftKind = (typeof DraftKind)[keyof typeof DraftKind];

/** A task's `status`. (tern-ipc `task_status`) */
export const TaskStatus = {
  Local: 'local',
  Remote: 'remote',
  Complete: 'complete',
  Cancelled: 'cancelled',
} as const;
export type TaskStatus = (typeof TaskStatus)[keyof typeof TaskStatus];

/** The bits of `MessageJson.folders` values. (tern-ipc `placement_flag`) */
export const PlacementFlag = {
  Unread: 1,
  Starred: 2,
  Draft: 4,
} as const;
export type PlacementFlag = (typeof PlacementFlag)[keyof typeof PlacementFlag];

/** The protocol version `hello` names. */
export const AI_TRIAGE_RUN_MAX = 200;
export const CREDENTIALS_PER_REQUEST = 'request';
export const DRAFT_ATTACHMENTS_MAX_BYTES = 25000000;
export const EVENTS_EXPAND_MAX_OCCURRENCES = 10000;
export const FILES_READ_MAX_BYTES = 4194304;
export const LOGS_TAIL_DEFAULT_BYTES = 65536;
export const LOGS_TAIL_MAX_BYTES = 1048576;
export const MAIL_RULES_AUTO_SINCE_KEY = 'mailRulesAutoSince';
export const MAIL_RULES_KEY = 'mailRules';
export const MAX_LINE_BYTES = 67108864;
export const PROTOCOL_VERSION = 1;
export const READ_DEFAULT_LIMIT = 50;
export const READ_MAX_LIMIT = 500;
export const SCOPE_ADMIN = 'admin';
export const THREAD_COUNTS_CLASS = 'ThreadCounts';

/** Every key `settings.get` and `settings.set` take, with the JSON type of its value. */
export const SETTINGS_KEYS = {
  'composing.quoteTexts': 'object',
  'core.ai.accounts': 'array',
  'core.ai.features': 'object',
  'core.ai.includeAddresses': 'boolean',
  'core.ai.limits': 'object',
  'core.ai.providers': 'array',
  'core.composing.html': 'boolean',
  'core.integrations.enrichment': 'object',
  'core.integrations.grammar': 'object',
  'core.integrations.translation': 'object',
  'core.notifications.unsnoozeToTop': 'boolean',
  'core.sending.defaultAccountIdForSend': 'string',
  'core.sending.undoSend': 'count',
  'defaultSignatures': 'object',
  'mailRules': 'array',
  'mailRulesAutoSince': 'count',
  'resurfacing.reminderBody': 'string',
  'resurfacing.snoozeBody': 'string',
  'signatures': 'object',
  'templates': 'array',
} as const;
export type SettingsKey = keyof typeof SETTINGS_KEYS;

/**
 * Who may call each method: `open` (any connection), `local-only` (local clients and
 * relayed devices with the admin scope), `parent-only` (the engine's parent), `debug`.
 */
export const METHOD_CLASSES = {
  'account.add': 'local-only',
  'account.discover': 'open',
  'account.list': 'open',
  'account.oauth.begin': 'local-only',
  'account.oauth.complete': 'local-only',
  'account.remove': 'local-only',
  'account.reorder': 'open',
  'account.reset': 'local-only',
  'account.setSecrets': 'local-only',
  'account.test': 'local-only',
  'account.update': 'open',
  'ai.cancel': 'open',
  'ai.draftReply': 'open',
  'ai.models': 'open',
  'ai.suggestReplies': 'open',
  'ai.summarize': 'open',
  'ai.test': 'open',
  'ai.triage.list': 'open',
  'ai.triage.run': 'open',
  'ai.usage': 'open',
  'bodies.fetch': 'open',
  'calendar.sync': 'open',
  'contacts.create': 'local-only',
  'contacts.export': 'open',
  'contacts.get': 'open',
  'contacts.import': 'local-only',
  'contacts.search': 'open',
  'contacts.update': 'local-only',
  'counts.get': 'open',
  'credentials.import': 'parent-only',
  'draft.cancelSend': 'open',
  'draft.create': 'open',
  'draft.delete': 'open',
  'draft.send': 'open',
  'draft.update': 'open',
  'draft.validate': 'open',
  'engine.logs.tail': 'local-only',
  'engine.resetDatabase': 'parent-only',
  'engine.setLogLevel': 'parent-only',
  'engine.shutdown': 'parent-only',
  'engine.status': 'open',
  'events.create': 'local-only',
  'events.expand': 'open',
  'events.rsvp': 'local-only',
  'events.update': 'local-only',
  'files.get': 'local-only',
  'files.put': 'local-only',
  'files.read': 'local-only',
  'folders.list': 'open',
  'hello': 'open',
  'integrations.detect': 'local-only',
  'integrations.enrich': 'local-only',
  'integrations.grammar': 'local-only',
  'integrations.languages': 'local-only',
  'integrations.test': 'local-only',
  'integrations.translate': 'local-only',
  'labels.list': 'open',
  'messages.get': 'open',
  'messages.unsubscribe': 'open',
  'metadata.list': 'open',
  'query': 'open',
  'rules.cancel': 'local-only',
  'rules.reprocess': 'local-only',
  'search': 'open',
  'secrets.delete': 'local-only',
  'secrets.has': 'local-only',
  'secrets.set': 'local-only',
  'server.clear': 'local-only',
  'server.set': 'local-only',
  'settings.get': 'open',
  'settings.set': 'open',
  'subscribe': 'open',
  'sync.wake': 'open',
  'task.cancel': 'open',
  'task.queue': 'open',
  'tasks.get': 'open',
  'tasks.list': 'open',
  'templates.delete': 'local-only',
  'templates.get': 'open',
  'templates.list': 'open',
  'templates.save': 'local-only',
  'test.crash': 'debug',
  'test.panic': 'debug',
  'threads.get': 'open',
  'threads.list': 'open',
  'threads.snooze': 'open',
  'threads.unsnooze': 'open',
} as const;
