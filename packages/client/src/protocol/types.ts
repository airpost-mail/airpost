// SPDX-License-Identifier: GPL-3.0-only
// Generated from crates/tern-ipc by packages/client/scripts/generate.mjs. Do not edit: change
// the Rust types, then run `npm run generate` in packages/client.

/**
 * Any model a `query`, read method or delta carries, by `__cls`.
 */
export type AnyModelJson =
  | ThreadJson
  | MessageJson
  | FolderJson
  | LabelJson
  | ContactJson
  | ContactGroupJson
  | ContactBookJson
  | CalendarJson
  | EventJson
  | FileJson
  | TaskJson;
/**
 * A folder's or label's role: one role detection assigns (tern-core `ROLES`), or empty for
 * a user folder or label. The engine doesn't check the role `ChangeRoleMappingTask` sets.
 */
export type CategoryRole =
  | ''
  | 'inbox'
  | 'important'
  | 'snoozed'
  | 'sent'
  | 'drafts'
  | 'all'
  | 'spam'
  | 'archive'
  | 'trash'
  | 'starred';
/**
 * Any task the engine runs, by `__cls`. Schema only: in Rust a task deserialises as the
 * struct its `__cls` names.
 */
export type TaskJson =
  | ChangeUnreadTaskJson
  | ChangeStarredTaskJson
  | ChangeFolderTaskJson
  | ChangeLabelsTaskJson
  | DestroyDraftTaskJson
  | SyncbackDraftTaskJson
  | SendDraftTaskJson
  | SyncbackMetadataTaskJson
  | ExpungeAllInFolderTaskJson
  | SyncbackCategoryTaskJson
  | DestroyCategoryTaskJson
  | ChangeRoleMappingTaskJson
  | GetMessageRFC2822TaskJson
  | GetManyRFC2822TaskJson
  | EventRSVPTaskJson
  | SyncbackContactTaskJson
  | DestroyContactTaskJson
  | SyncbackContactGroupTaskJson
  | DestroyContactGroupTaskJson
  | ChangeContactGroupMembershipTaskJson
  | SyncbackEventTaskJson
  | DestroyEventTaskJson;
/**
 * Which occurrences an `events.update` changes.
 */
export type EventScope = 'this' | 'following' | 'all';
/**
 * One step of a `query.changed`, applied in order to the rows the client holds: `remove`
 * drops the row with `id`; `insert` puts `row` at `index`; `move` takes the row with `id`
 * out and puts it back at `index`; `update` replaces the row with `row`'s id in place. An
 * index counts the rows as they are after the steps before it.
 */
export type LiveOp =
  | {
      id: string;
      op: 'remove';
    }
  | {
      index: number;
      op: 'insert';
      row: AnyModelJson;
    }
  | {
      id: string;
      index: number;
      op: 'move';
    }
  | {
      op: 'update';
      row: AnyModelJson;
    };
/**
 * A matcher as `Matcher.toJSON()` writes it: `attr` is the desktop's model key
 * (`lastMessageReceivedTimestamp`), not the JSON key. Schema only.
 */
export type MatcherJson =
  | {
      attr: string;
      op: QueryOp;
      value: unknown;
    }
  | {
      and: MatcherJson[];
    }
  | {
      or: MatcherJson[];
    }
  | {
      not: MatcherJson[];
    };
/**
 * The operators of a matcher (desktop `src/data/attributes/matcher.ts`). Schema only.
 */
export type QueryOp =
  | '='
  | '!='
  | '<'
  | '>'
  | '<='
  | '>='
  | 'in'
  | 'not in'
  | 'contains'
  | 'containsAny'
  | 'startsWith'
  | 'like';
/**
 * A `metadata.list` model: a Thread or a Message, by `__cls`.
 */
export type MetadataModelJson = ThreadJson | MessageJson;
/**
 * The classes `query` reads (`crate::query_attrs::QUERY_CLASSES`). Schema only.
 */
export type QueryClassName =
  | 'Thread'
  | 'Message'
  | 'Folder'
  | 'Label'
  | 'Contact'
  | 'ContactGroup'
  | 'ContactBook'
  | 'Event'
  | 'Calendar'
  | 'Task'
  | 'File';
/**
 * `query.where`: one matcher, or a list that must all match. Schema only.
 */
export type QueryWhere = MatcherJson | MatcherJson[];

/**
 * `account.add` params. The account JSON carries credentials, which is why the method is
 * only accepted on the parent's stdin.
 */
export interface AccountAddParams {
  /**
   * The account JSON `sync` mode reads from `--account` or stdin. Without an `id` the
   * engine derives one (`ACCOUNT_ID_SETTINGS`).
   */
  account: AccountJson;
  /**
   * The identity line `sync` mode reads: a Tern server object, a Mailspring identity
   * (ignored) or null. Null uses the server `server.set` stored, if any.
   */
  identity?: unknown;
  /**
   * Secrets to store with the account, in place of any in its `settings`.
   */
  secrets?: AccountSecrets | null;
  /**
   * Run `account.test` first and add the account only when it passes; a failure is
   * `code::ACCOUNT_TEST_FAILED` with the test's result.
   */
  test?: boolean;
}
/**
 * An account's secrets, under the `settings` keys they replace. Write-only: no method
 * returns them.
 */
export interface AccountSecrets {
  imap_password?: string;
  refresh_token?: string;
  smtp_password?: string;
}
/**
 * `account.add` result: the account's state when the call returns, `running` once its
 * `main` thread is up, `stopped` (with `error`) when it failed to start.
 */
export interface AccountAddResult {
  accountId: string;
  error?: SyncErrorJson;
  state: 'starting' | 'running' | 'stopped' | 'removed';
  /**
   * The account and its secrets are in the engine's registry and secret store, so the
   * client need not keep them (`feature::ACCOUNTS`). False
   * when no secret store could take them: the engine runs the account until it exits.
   */
  stored?: boolean;
}
/**
 * The engine's `SyncError` JSON: why an account stopped or a task failed.
 */
export interface SyncErrorJson {
  debuginfo?: string;
  /**
   * An `ErrorCodeToTypeMap` key, e.g. `ErrorAuthentication`.
   */
  key: string;
  offline?: boolean;
  retryable?: boolean;
  what?: string;
}
/**
 * `account.changed` params.
 */
export interface AccountChangedParams {
  /**
   * As `AccountStatus::account`.
   */
  account: AccountJson;
  accountId: string;
}
/**
 * `account.discover` params.
 */
export interface AccountDiscoverParams {
  email: string;
  /**
   * The provider preset the user picked: `imap` (default), `gmail`, `office365`,
   * `outlook`, `yahoo`, `icloud`, `fastmail`, …
   */
  provider?: string | null;
}
/**
 * `account.discover` result.
 */
export interface AccountDiscoverResult {
  /**
   * `{emailAddress, provider, settings}`: hosts, ports, usernames, security,
   * `*_allow_insecure_ssl` and `container_folder`. Security is never `none`. An empty
   * `imap_host` means nothing was found that offers IMAP; the client asks for the servers.
   */
  account: DiscoveredAccountJson;
  /**
   * `matcher`, `autoconfig`, `provider-table` or `fallback` (the `imap.<domain>` guess).
   */
  source: string;
  /**
   * The table entry used, when one was.
   */
  template?: string;
}
/**
 * `account.remove`, `test.crash` and `test.panic` params.
 */
export interface AccountIdParams {
  accountId: string;
}
export interface AccountInfo {
  accountId: string;
  /**
   * As in `AccountStateParams::auth_failed`.
   */
  authFailed?: boolean;
  email: string;
  /**
   * The `SyncError` JSON that stopped the account, when `state` is `stopped`.
   */
  error?: SyncErrorJson;
  /**
   * As in `AccountStateParams::retry_at`.
   */
  retryAt?: number;
  /**
   * One of `account_state`.
   */
  state: 'starting' | 'running' | 'stopped' | 'removed';
}
/**
 * The account JSON without secrets, as `account.list`, `account.changed` and the account
 * methods return it. On a relayed connection only the fields
 * `public_account_json` keeps.
 */
export interface AccountJson {
  __cls?: string;
  aid?: string;
  aliases?: string[];
  authedAt?: number;
  autoaddress?: Autoaddress;
  color?: string;
  defaultAlias?: string | null;
  emailAddress: string;
  id: string;
  label?: string;
  metadata?: PluginMetadataJson[];
  name?: string;
  /**
   * `gmail`, `office365`, `outlook`, `imap`, ...
   */
  provider?: string;
  /**
   * IMAP and SMTP settings; never the passwords or the refresh token.
   */
  settings?: AccountSettingsJson;
  syncError?: unknown;
  /**
   * `ok`, `invalid` or `sync_error`.
   */
  syncState?: string;
  v?: number;
}
/**
 * An account's automatic Cc or Bcc (`AccountJson.autoaddress`).
 */
export interface Autoaddress {
  type: 'cc' | 'bcc';
  /**
   * Addresses separated by `,` or `;`.
   */
  value: string;
}
/**
 * One plugin's metadata on a model (`ModelWithMetadata.pluginMetadata`).
 */
export interface PluginMetadataJson {
  pluginId: string;
  v: number;
  value: unknown;
}
/**
 * `account.list` result.
 */
export interface AccountListResult {
  accounts: AccountStatus[];
}
export interface AccountStatus {
  /**
   * The account JSON without its secrets: on a local connection everything but the
   * passwords and refresh token; on a relayed one only the fields
   * `public_account_json` lists.
   */
  account?: AccountJson;
  accountId: string;
  /**
   * As in `AccountStateParams::auth_failed`.
   */
  authFailed?: boolean;
  /**
   * The last `ProcessState` the account sent: true while it cannot reach its server.
   */
  connectionError: boolean;
  email: string;
  /**
   * The `SyncError` JSON that stopped the account, when `state` is `stopped`.
   */
  error?: SyncErrorJson;
  /**
   * As in `AccountStateParams::retry_at`.
   */
  retryAt?: number;
  state: 'starting' | 'running' | 'stopped' | 'removed';
}
/**
 * `account.remove` params.
 */
export interface AccountRemoveParams {
  accountId: string;
  /**
   * Delete the account's rows from the database, plugin metadata included.
   */
  deleteData?: boolean;
  /**
   * Revoke the account's OAuth grant at the provider (Google; Microsoft has no such call).
   */
  revoke?: boolean;
}
/**
 * `account.remove` result.
 */
export interface AccountRemoveResult {
  /**
   * The names of the account's threads still running after the stop timeout, left to
   * finish a blocking call (e.g. `background`). Empty when every thread exited.
   */
  stillRunning?: string[];
}
/**
 * `account.reorder` params: account ids in the order `account.list` returns them. Ids not
 * named keep their order after the named ones.
 */
export interface AccountReorderParams {
  ids: string[];
}
/**
 * `account.reset` params. `keepMetadata` (default true) keeps the account's plugin metadata
 * for the resync; a client removing the account passes false so it goes with the rest.
 */
export interface AccountResetParams {
  accountId: string;
  keepMetadata?: boolean;
}
/**
 * `account.setSecrets` params: new credentials for a registered account (reconnect). The
 * account restarts with them.
 */
export interface AccountSetSecretsParams {
  accountId: string;
  secrets: AccountSecrets;
  /**
   * Test them first, as `AccountAddParams::test`.
   */
  test?: boolean;
}
export interface AccountSettingsJson {
  container_folder?: string;
  /**
   * The host the user confirmed `none` for.
   */
  imap_allow_cleartext?: string;
  imap_allow_insecure_ssl?: boolean;
  imap_host?: string;
  /**
   * Only in what a client sends (`account.add`, `account.test`); never returned.
   */
  imap_password?: string;
  imap_port?: number;
  /**
   * `SSL / TLS`, `STARTTLS` or `none`.
   */
  imap_security?: string;
  imap_username?: string;
  /**
   * As `imap_password`.
   */
  refresh_token?: string;
  smtp_allow_cleartext?: string;
  smtp_allow_insecure_ssl?: boolean;
  smtp_host?: string;
  /**
   * As `imap_password`.
   */
  smtp_password?: string;
  smtp_port?: number;
  smtp_security?: string;
  smtp_username?: string;
}
/**
 * `account.state` notification params.
 */
export interface AccountStateParams {
  accountId: string;
  /**
   * With `stopped`: the server refused the credentials. The engine doesn't retry; new
   * credentials (`account.add`) do.
   */
  authFailed?: boolean;
  /**
   * The `SyncError` JSON, with `stopped`.
   */
  error?: SyncErrorJson;
  /**
   * With `stopped`: when the engine starts the account again (Unix milliseconds). Absent
   * once it gave up (too many failures in a row) or the credentials were refused.
   */
  retryAt?: number;
  /**
   * One of `account_state`.
   */
  state: 'starting' | 'running' | 'stopped' | 'removed';
}
/**
 * `account.test` params: the account JSON with its credentials in `settings` or in
 * `secrets`.
 */
export interface AccountTestParams {
  account: AccountJson;
  secrets?: AccountSecrets | null;
}
/**
 * `account.test`'s result, `--mode test`'s object: `account` (with any rotated refresh
 * token, on stdin only) on success, `error` (an `ErrorCodeToTypeMap` name) and `log` on
 * failure.
 */
export interface AccountTestResult {
  account?: AccountJson | null;
  error?: string | null;
  /**
   * After a TLS failure: what the user can change.
   */
  error_advice?: string;
  /**
   * The OAuth refresh failed because the network is down.
   */
  error_offline?: boolean;
  /**
   * `imap` or `smtp`: the server that failed.
   */
  error_service?: string;
  log: string;
}
/**
 * `account.update` params. `patch` holds top-level Account fields to replace and a
 * `settings` object whose keys replace the account's (null removes one). Secrets and `id`
 * can't be patched. Changing where the account connects (host, port, username, security,
 * provider or OAuth client) deletes its stored secrets, so they can't be sent to the new
 * server; the account stays stopped until `account.setSecrets`.
 */
export interface AccountUpdateParams {
  accountId: string;
  patch: unknown;
}
export interface AccountUpdateResult {
  /**
   * The account's JSON without secrets.
   */
  account: AccountJson;
  /**
   * The running account restarted with the new settings.
   */
  restarted?: boolean;
  /**
   * The stored secrets were deleted: send new ones with `account.setSecrets`.
   */
  secretsCleared?: boolean;
}
/**
 * `ai.cancel`: one of the two.
 */
export interface AiCancelParams {
  jobId?: string | null;
  streamId?: string | null;
}
/**
 * `ai.summarize` and `ai.draftReply` (tern-ai `Completion`): plain text, never rendered as
 * HTML. `cached` only on summaries.
 */
export interface AiCompletion {
  cached?: boolean;
  /**
   * The model that answered, as the provider named it.
   */
  model: string;
  text: string;
  usage?: AiTokenUsage;
}
/**
 * The tokens of one request (tern-ai `TokenUsage`).
 */
export interface AiTokenUsage {
  /**
   * The provider gave no counts; four characters per token.
   */
  estimated?: boolean;
  inputTokens: number;
  outputTokens: number;
}
export interface AiDeltaParams {
  streamId: string;
  text: string;
}
/**
 * Which provider a call uses: `config`, the whole provider config with its key (local
 * clients, until the engine holds the keys), or `provider`, the id of one the engine keeps.
 * Its `Debug` hides `config`, which holds the key.
 */
export interface AiDraftReplyParams {
  accountId: string;
  config?: unknown;
  /**
   * What the user typed ("say I'll call Tuesday").
   */
  instruction?: string | null;
  provider?: string | null;
  settings?: AiSettings | null;
  streamId?: string | null;
  /**
   * The text of the suggestion the user picked.
   */
  suggestion?: string | null;
  threadId: string;
}
/**
 * The AI switches a local client sends with each call until the engine keeps them:
 * the accounts opted in, the features turned on, and whether the
 * model may see addresses. The engine checks them before it reads any mail.
 */
export interface AiSettings {
  /**
   * Account ids.
   */
  accounts?: string[];
  /**
   * By `ai_feature`: `true`, or an object whose `enabled` is `true`.
   */
  features?: {
    [k: string]: unknown;
  };
  includeAddresses?: boolean;
}
/**
 * One `ai.models` row (tern-ai `ModelInfo`).
 */
export interface AiModelInfo {
  contextTokens?: number;
  id: string;
  name: string;
}
/**
 * Which provider a call uses: `config`, the whole provider config with its key (local
 * clients, until the engine holds the keys), or `provider`, the id of one the engine keeps.
 * Its `Debug` hides `config`, which holds the key.
 */
export interface AiProviderParams {
  config?: unknown;
  provider?: string | null;
}
/**
 * Which provider a call uses: `config`, the whole provider config with its key (local
 * clients, until the engine holds the keys), or `provider`, the id of one the engine keeps.
 * Its `Debug` hides `config`, which holds the key.
 */
export interface AiSuggestRepliesParams {
  accountId: string;
  config?: unknown;
  /**
   * 3 when absent, at most 3.
   */
  count?: number | null;
  provider?: string | null;
  settings?: AiSettings | null;
  /**
   * Chosen by the client so `ai.cancel` can stop the call. Suggestions don't stream: no
   * `ai.delta` carries it.
   */
  streamId?: string | null;
  threadId: string;
}
/**
 * One `ai.suggestReplies` row (tern-ai `Suggestion`): a one-line reply the user can pick.
 */
export interface AiSuggestion {
  label: string;
  text: string;
}
/**
 * Which provider a call uses: `config`, the whole provider config with its key (local
 * clients, until the engine holds the keys), or `provider`, the id of one the engine keeps.
 * Its `Debug` hides `config`, which holds the key.
 */
export interface AiSummarizeParams {
  accountId: string;
  config?: unknown;
  provider?: string | null;
  /**
   * Ask again even when a summary is cached.
   */
  refresh?: boolean;
  settings?: AiSettings | null;
  /**
   * Chosen by the client; the text streams as `ai.delta` with it.
   */
  streamId?: string | null;
  threadId: string;
}
/**
 * `ai.test`'s answer (tern-ai `TestReport`): where requests go and how long a minimal one
 * took.
 */
export interface AiTestReport {
  host: string;
  latencyMs: number;
  /**
   * The provider is on a loopback or private address.
   */
  local: boolean;
  model: string;
}
/**
 * `ai.triage.list`: the opted-in accounts' inbox threads with a triage result, newest
 * first. `category` is one of the triage categories; `needs-reply` also takes threads whose
 * result says `needsReply` under another category.
 */
export interface AiTriageListParams {
  accountIds?: string[] | null;
  category?: string | null;
  /**
   * At most `READ_MAX_LIMIT`.
   */
  limit: number;
  offset?: number | null;
}
export interface AiTriageListResult {
  threadIds: string[];
  /**
   * Every match, not just this page.
   */
  total: number;
}
/**
 * `ai.triage.progress`, to the connection that called `ai.triage.run`: `done` of `total`
 * threads handled, triaged or skipped. The last one has `done == total`, or `stopped`.
 */
export interface AiTriageProgressParams {
  done: number;
  jobId: string;
  /**
   * The job ended after `done` threads: it was cancelled, the account stopped, triage
   * was turned off for it, or the provider it named went away.
   */
  stopped?: boolean;
  total: number;
}
/**
 * `ai.triage.run`: the threads to triage, by id or as the inbox threads of the last
 * `sinceDays` days; at most `AI_TRIAGE_RUN_MAX` either way. The provider is the one the
 * engine keeps for triage unless the call names one (`provider`) or, from a local client,
 * carries one with its key (`config`).
 */
export interface AiTriageRunParams {
  accountId: string;
  config?: unknown;
  provider?: string | null;
  sinceDays?: number | null;
  threadIds?: string[] | null;
}
export interface AiTriageRunResult {
  /**
   * Named by `ai.triage.progress` and `ai.cancel {jobId}`.
   */
  jobId: string;
  /**
   * How many threads the job holds.
   */
  queued: number;
}
export interface AiUsageParams {
  /**
   * 30 when absent.
   */
  days?: number | null;
}
/**
 * One row of `ai.usage`'s result, an array newest day first.
 */
export interface AiUsageRow {
  /**
   * `YYYY-MM-DD` in the engine's local time.
   */
  day: string;
  /**
   * From `ai_feature`, or `test`.
   */
  feature: string;
  inputTokens: number;
  outputTokens: number;
  /**
   * The provider config's id.
   */
  provider: string;
  requests: number;
}
export interface ThreadJson {
  __cls: 'Thread';
  aid: string;
  attachmentCount: number;
  /**
   * firstMessageTimestamp.
   */
  fmt: number;
  folders: ThreadCategoryRef[];
  /**
   * The Gmail thread id (X-GM-THRID) as a string; empty on IMAP accounts.
   */
  gThrId?: string;
  id: string;
  inAllMail: boolean;
  labels: ThreadCategoryRef[];
  /**
   * lastMessageReceivedTimestamp.
   */
  lmrt: number;
  /**
   * `lmrt` came from a sent message because none was received; engine bookkeeping.
   */
  lmrt_is_fallback?: boolean;
  /**
   * lastMessageSentTimestamp.
   */
  lmst: number;
  /**
   * lastMessageTimestamp: any message.
   */
  lmt: number;
  metadata?: PluginMetadataJson[];
  participants: ContactRef[];
  /**
   * The thread's `ThreadSearch` row; engine bookkeeping.
   */
  searchRowId?: number;
  /**
   * Messages that are starred.
   */
  starred: number;
  subject: string;
  /**
   * Messages that are unread (a count; the desktop reads it as a boolean).
   */
  unread: number;
  v: number;
}
/**
 * A folder or label as a Thread embeds it in `folders` and `labels`.
 */
export interface ThreadCategoryRef {
  /**
   * `Folder` or `Label`.
   */
  __cls: 'Folder' | 'Label';
  /**
   * The thread's messages in the category.
   */
  _refs: number;
  /**
   * The thread's unread messages in the category.
   */
  _u: number;
  aid: string;
  id: string;
  path: string;
  role: CategoryRole;
  v?: number;
}
/**
 * A contact as embedded in messages and threads (`from`, `to`, `participants`, ...).
 */
export interface ContactRef {
  email: string;
  name?: string;
}
export interface MessageJson {
  __cls: 'Message';
  /**
   * The sync timestamp of the headers (syncedAt); engine bookkeeping.
   */
  _sa?: number;
  /**
   * The sync's unchanged count (syncUnsavedChanges); engine bookkeeping.
   */
  _suc?: number;
  aid: string;
  bcc: ContactRef[];
  /**
   * From MessageBody: on `query` with `include: ["body"]`, and on deltas once fetched.
   */
  body?: string | null;
  cc: ContactRef[];
  date: number;
  draft: boolean;
  events?: unknown[];
  extraHeaders?: {
    [k: string]: unknown;
  };
  files: FileJson[];
  /**
   * Folder id -> `placement_flag` bits, one key per copy on the server. The engine
   * always writes it; a draft a client sends may leave it out.
   */
  folders?: {
    [k: string]: number;
  };
  from: ContactRef[];
  /**
   * Deltas only: the body has been fetched.
   */
  fullSyncComplete?: boolean;
  /**
   * forwardedHeaderMessageId.
   */
  fwdMsgId?: string | null;
  /**
   * The Gmail message id as a string.
   */
  gMsgId?: string;
  /**
   * List-Unsubscribe.
   */
  hListUnsub?: string;
  /**
   * List-Unsubscribe-Post.
   */
  hListUnsubPost?: string;
  /**
   * headerMessageId.
   */
  hMsgId: string;
  /**
   * Deltas only: the first save of a message's headers.
   */
  headersSyncComplete?: boolean;
  /**
   * `Message.isHidden()`, on the rows of `query` and the read methods
   * (`feature::MESSAGE_HIDDEN`).
   */
  hidden?: boolean;
  id: string;
  /**
   * X-GM-LABELS values (`\\Inbox`, `Work`); as `folders`, absent on a client's draft.
   */
  labels?: string[];
  metadata?: PluginMetadataJson[];
  /**
   * `body` is plain text rather than HTML.
   */
  plaintext?: boolean;
  pristine?: boolean;
  replyTo: ContactRef[];
  /**
   * Whether the mail rules still wait for the body; engine bookkeeping.
   */
  rr?: number;
  /**
   * replyToHeaderMessageId.
   */
  rthMsgId?: string | null;
  /**
   * Deltas only: the mail rules have run on the message.
   */
  rulesReady?: boolean;
  snippet?: string;
  starred: boolean;
  subject: string;
  threadId: string;
  to: ContactRef[];
  unread: boolean;
  v: number;
  /**
   * On `messages.get` and `threads.get` (`feature::MESSAGE_WARNINGS`),
   * left out when there are none.
   */
  warnings?: MessageWarning[];
}
export interface FileJson {
  __cls: 'File';
  aid: string;
  /**
   * Set for inline parts, matching `cid:` references in the HTML body.
   */
  contentId?: string | null;
  contentType: string;
  filename: string;
  id: string;
  messageId: string;
  partId?: string;
  size: number;
  v: number;
}
/**
 * One warning about a message (`warnings` on the Message JSON of `messages.get` and
 * `threads.get`, `feature::MESSAGE_WARNINGS`).
 */
export interface MessageWarning {
  /**
   * `reply-to-mismatch`: replies go to another domain than the message came from (the
   * desktop's phishing-detection plugin).
   */
  code: string;
  /**
   * The desktop's English text.
   */
  message: string;
}
/**
 * An IMAP mailbox.
 */
export interface FolderJson {
  __cls: 'Folder';
  aid: string;
  id: string;
  /**
   * Sync state (`uidvalidity`, `highestmodseq`, ...); left out of the copies a
   * Thread embeds.
   */
  localStatus?: {
    [k: string]: unknown;
  };
  /**
   * The IMAP path, in modified UTF-7.
   */
  path: string;
  role: CategoryRole;
  v: number;
}
/**
 * A Gmail label.
 */
export interface LabelJson {
  __cls: 'Label';
  aid: string;
  id: string;
  /**
   * Sync state (`uidvalidity`, `highestmodseq`, ...); left out of the copies a
   * Thread embeds.
   */
  localStatus?: {
    [k: string]: unknown;
  };
  /**
   * The IMAP path, in modified UTF-7.
   */
  path: string;
  role: CategoryRole;
  v: number;
}
export interface ContactJson {
  __cls: 'Contact';
  aid: string;
  /**
   * The ContactBook id.
   */
  bid?: string;
  email: string;
  etag?: string;
  /**
   * The ContactGroup ids.
   */
  gis: string[];
  /**
   * The provider's resource name.
   */
  grn?: string;
  /**
   * hidden.
   */
  h?: boolean;
  id: string;
  /**
   * The provider's record: `{vcf}` for CardDAV, the person JSON for Google.
   */
  info: unknown;
  name: string;
  /**
   * How many messages name the contact.
   */
  refs: number;
  /**
   * source: `mail`, `carddav` or `gpeople`.
   */
  s: string;
  /**
   * The contact's `ContactSearch` row; engine bookkeeping.
   */
  searchRowId?: number;
  v: number;
}
export interface ContactGroupJson {
  __cls: 'ContactGroup';
  aid: string;
  /**
   * The ContactBook id.
   */
  bid?: string;
  /**
   * The provider's resource name.
   */
  grn?: string;
  id: string;
  name: string;
  v: number;
}
export interface ContactBookJson {
  __cls: 'ContactBook';
  aid: string;
  ctag?: string;
  id: string;
  /**
   * `carddav` or `gpeople`.
   */
  source: string;
  syncToken?: string;
  url: string;
  v: number;
}
/**
 * A CalDAV collection.
 */
export interface CalendarJson {
  __cls: 'Calendar';
  aid: string;
  /**
   * `#rrggbb`, lower case.
   */
  color?: string;
  ctag?: string;
  description?: string;
  id: string;
  name: string;
  order?: number;
  owner?: string;
  path?: string;
  read_only?: boolean;
  syncToken?: string;
  v: number;
}
/**
 * A stored VEVENT (one per master or recurrence exception).
 */
export interface EventJson {
  __cls: 'Event';
  aid: string;
  /**
   * calendarId.
   */
  cid: string;
  etag?: string;
  href?: string;
  ics: string;
  icsuid: string;
  id: string;
  /**
   * recurrenceEnd: the last occurrence's end.
   */
  re: number;
  /**
   * recurrenceId; empty on a master.
   */
  rid?: string;
  /**
   * recurrenceStart: the first occurrence's start.
   */
  rs: number;
  /**
   * The event's `EventSearch` row; engine bookkeeping.
   */
  searchRowId?: number;
  status?: string;
  v: number;
}
export interface ChangeUnreadTaskJson {
  __cls: 'ChangeUnreadTask';
  aid: string;
  canBeUndone?: boolean;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  id: string;
  isUndo?: boolean;
  messageIds: string[];
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  taskDescription?: string;
  threadIds: string[];
  unread: boolean;
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface ChangeStarredTaskJson {
  __cls: 'ChangeStarredTask';
  aid: string;
  canBeUndone?: boolean;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  id: string;
  isUndo?: boolean;
  messageIds: string[];
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  starred: boolean;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  taskDescription?: string;
  threadIds: string[];
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface ChangeFolderTaskJson {
  __cls: 'ChangeFolderTask';
  aid: string;
  canBeUndone?: boolean;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  folder: FolderJson;
  id: string;
  isUndo?: boolean;
  messageIds: string[];
  /**
   * An undo: the placements to put back.
   */
  restorePlacements?: unknown;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  sourceFolderIds?: string[];
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  taskDescription?: string;
  threadIds: string[];
  /**
   * Set by the local phase: where each message was, for an undo.
   */
  undoPlacements?: unknown;
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface ChangeLabelsTaskJson {
  __cls: 'ChangeLabelsTask';
  aid: string;
  canBeUndone?: boolean;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  id: string;
  isUndo?: boolean;
  labelsToAdd: LabelJson[];
  labelsToRemove: LabelJson[];
  messageIds: string[];
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  taskDescription?: string;
  threadIds: string[];
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface DestroyDraftTaskJson {
  __cls: 'DestroyDraftTask';
  aid: string;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  id: string;
  messageIds: string[];
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface SyncbackDraftTaskJson {
  __cls: 'SyncbackDraftTask';
  aid: string;
  draft: MessageJson;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  headerMessageId: string;
  id: string;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface SendDraftTaskJson {
  __cls: 'SendDraftTask';
  /**
   * When the send started waiting for the network (unix seconds).
   */
  _offlineSince?: number;
  aid: string;
  draft: MessageJson;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  headerMessageId: string;
  id: string;
  /**
   * Recipient email -> the body sent to them.
   */
  perRecipientBodies?: {
    [k: string]: string;
  };
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * No notification when it's sent (a send-later or a mail-rule forward).
   */
  silent?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface SyncbackMetadataTaskJson {
  __cls: 'SyncbackMetadataTask';
  aid: string;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  id: string;
  /**
   * `Thread` or `Message`.
   */
  modelClassName: string;
  /**
   * `null` for a model that isn't a Message.
   */
  modelHeaderMessageId?: string | null;
  modelId: string;
  pluginId: string;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  undoValue?: unknown;
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
  value: unknown;
}
export interface ExpungeAllInFolderTaskJson {
  __cls: 'ExpungeAllInFolderTask';
  aid: string;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  folder: FolderJson;
  id: string;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface SyncbackCategoryTaskJson {
  __cls: 'SyncbackCategoryTask';
  aid: string;
  /**
   * Set by the remote phase: the category created or renamed.
   */
  created?: unknown;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  /**
   * A rename: the folder or label's current path.
   */
  existingPath?: string;
  id: string;
  path: string;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface DestroyCategoryTaskJson {
  __cls: 'DestroyCategoryTask';
  aid: string;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  id: string;
  path: string;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface ChangeRoleMappingTaskJson {
  __cls: 'ChangeRoleMappingTask';
  aid: string;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  id: string;
  path: string;
  role: CategoryRole;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface GetMessageRFC2822TaskJson {
  __cls: 'GetMessageRFC2822Task';
  aid: string;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  /**
   * Set by the local phase in file-id mode.
   */
  fileId?: string;
  /**
   * Local clients only. Without it (file-id mode) the engine saves the File `fileId`.
   */
  filepath?: string;
  id: string;
  messageId: string;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface GetManyRFC2822TaskJson {
  __cls: 'GetManyRFC2822Task';
  aid: string;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  folderId: string;
  folderPath?: string;
  /**
   * `eml` or `mbox`.
   */
  format?: string;
  id: string;
  mboxPath?: string;
  outputDir?: string;
  progress?: unknown;
  result?: unknown;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface EventRSVPTaskJson {
  __cls: 'EventRSVPTask';
  aid: string;
  comment?: string;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  ics: string;
  icsRSVPStatus: string;
  id: string;
  messageId: string;
  method?: string;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  subject: string;
  to: string;
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface SyncbackContactTaskJson {
  __cls: 'SyncbackContactTask';
  aid: string;
  contact: ContactJson;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  id: string;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface DestroyContactTaskJson {
  __cls: 'DestroyContactTask';
  aid: string;
  contacts: ContactJson[];
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  id: string;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface SyncbackContactGroupTaskJson {
  __cls: 'SyncbackContactGroupTask';
  aid: string;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  group: ContactGroupJson;
  id: string;
  previousName?: string;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface DestroyContactGroupTaskJson {
  __cls: 'DestroyContactGroupTask';
  aid: string;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  group: ContactGroupJson;
  id: string;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface ChangeContactGroupMembershipTaskJson {
  __cls: 'ChangeContactGroupMembershipTask';
  aid: string;
  contacts: ContactJson[];
  /**
   * `add` or `remove`.
   */
  direction: string;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  group: ContactGroupJson;
  id: string;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface SyncbackEventTaskJson {
  __cls: 'SyncbackEventTask';
  aid: string;
  calendarId: string;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  event: EventJson;
  id: string;
  newData?: unknown;
  /**
   * An undo or redo, re-sent over the copy stored now.
   */
  restore?: boolean;
  rsvp?: unknown;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  taskDescription?: string;
  tz?: string;
  undoData?: unknown;
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface DestroyEventTaskJson {
  __cls: 'DestroyEventTask';
  aid: string;
  /**
   * The `SyncError` JSON of a failed task.
   */
  error?: SyncErrorJson | null;
  events: EventJson[];
  id: string;
  /**
   * `task.cancel` asked for it.
   */
  should_cancel?: boolean;
  /**
   * Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
   */
  source?: string;
  /**
   * One of `task_status`.
   */
  status: 'local' | 'remote' | 'complete' | 'cancelled';
  /**
   * Set by the engine when it stores the task; clients leave it out.
   */
  v?: number;
}
export interface BodiesFetchParams {
  accountId: string;
  /**
   * Message ids.
   */
  ids: string[];
}
export interface CalendarSyncParams {
  accountId: string;
}
export interface CalendarSyncResult {
  /**
   * False when a pass started by an earlier call is still running.
   */
  started: boolean;
}
export interface Capabilities {
  /**
   * By `ai_feature`: the AI features
   * this engine serves. A feature it doesn't serve yet is absent.
   */
  ai?: {
    [k: string]: Capability;
  };
  /**
   * By `integration_kind`.
   */
  integrations?: {
    [k: string]: Capability;
  };
  /**
   * By `server_feature`.
   */
  server?: {
    [k: string]: Capability;
  };
}
export interface Capability {
  available: boolean;
  /**
   * `CREDENTIALS_PER_REQUEST`, or absent when the engine needs none from the client.
   */
  credentials?: string;
  /**
   * Whether the provider runs on this computer or network (AI entries), which the
   * privacy note names.
   */
  local?: boolean;
  /**
   * The provider the user chose, e.g. `deepl`.
   */
  provider?: string;
  /**
   * Why it is unavailable, from `capability_reason`; absent when available.
   */
  reason?: string;
}
/**
 * `folders.list` and `labels.list`.
 */
export interface CategoriesListParams {
  accountId: string;
}
/**
 * A category's `ThreadCounts` row.
 */
export interface CategoryCounts {
  total: number;
  unread: number;
}
/**
 * `contacts.get`'s answer, and `contacts.create`'s and `contacts.update`'s.
 */
export interface ContactDetails {
  /**
   * The Contact JSON.
   */
  contact: ContactJson;
  data: unknown;
  /**
   * `{origin, readonly}`.
   */
  metadata: ContactOrigin;
  /**
   * The `SyncbackContactTask`, for a write.
   */
  taskId?: string;
}
/**
 * `ContactDetails::metadata`: where the contact comes from (`mail`, `carddav`, `gpeople`)
 * and whether it can be edited.
 */
export interface ContactOrigin {
  origin: string;
  readonly: boolean;
}
/**
 * `contacts.create`: a new contact in the account's address book (CardDAV, or Google
 * People on a Google account), with `data` in the `contacts.get` shape.
 */
export interface ContactsCreateParams {
  accountId: string;
  data: {
    [k: string]: unknown;
  };
}
/**
 * `contacts.export`: the contacts as one .vcf file, CardDAV cards as stored.
 */
export interface ContactsExportParams {
  accountId: string;
  ids: string[];
}
export interface ContactsExportResult {
  vcf: string;
}
/**
 * `contacts.get`: a contact's details in the shape the contact editor shows (`name`,
 * `emailAddresses`, `phoneNumbers`, `addresses`, `company`, `title`, `birthdays`, `urls`,
 * `nicknames`, `relations`, `notes`, `photoURL`), and where they come from.
 */
export interface ContactsGetParams {
  accountId: string;
  contactId: string;
}
/**
 * `contacts.import`: the vCards of a .vcf file as new CardDAV contacts.
 */
export interface ContactsImportParams {
  accountId: string;
  vcf: string;
}
export interface ContactsImportResult {
  contactIds: string[];
  taskIds: string[];
}
/**
 * `contacts.search` (over `ContactSearch`): plain text, each term matching a word prefix
 * of the contact's email or name.
 */
export interface ContactsSearchParams {
  accountId: string;
  limit?: number | null;
  query: string;
}
/**
 * `contacts.search`: matching contacts that aren't hidden, most referenced first.
 */
export interface ContactsSearchResult {
  contacts: ContactJson[];
}
/**
 * `contacts.update`: the fields of `patch` (the `contacts.get` shape, top-level keys
 * replacing the stored ones) written to the contact's vCard or Google person.
 */
export interface ContactsUpdateParams {
  accountId: string;
  contactId: string;
  patch: {
    [k: string]: unknown;
  };
}
/**
 * `counts.get`. `accounts` null (or absent) means every account the engine holds.
 */
export interface CountsGetParams {
  accounts?: string[] | null;
}
/**
 * Every `ThreadCounts` row of the accounts' folders and labels, keyed by category id. The
 * same rows arrive as `ThreadCounts` deltas (`{id, categoryId, unread, total}`, `persist`
 * when a count changes or a category is added, `unpersist` when it is removed), so a client
 * reads this once per connection and applies the deltas after it.
 */
export interface CountsGetResult {
  counts: {
    [k: string]: CategoryCounts;
  };
}
/**
 * `credentials.import` params: accounts and secrets a client stored until now. An
 * account's secrets come from its own `settings` (`imap_password`, `smtp_password`,
 * `refresh_token`), else from `secrets` under the client's email-keyed names
 * (`<email>-imap`, `<email>-smtp`, `<email>-refresh-token`), which go to every account with
 * that email. `integration-<provider>-key` and `tern-server-token` in `secrets` are stored
 * as `integration:<provider>` and `tern-server-token`; other names are ignored.
 */
export interface CredentialsImportParams {
  accounts?: unknown[];
  secrets?: {
    [k: string]: string;
  };
}
/**
 * `credentials.import` result. The imported accounts are registered, not started: the
 * client adds them (`account.add` without secrets) as it would have.
 */
export interface CredentialsImportResult {
  failed: ImportFailure[];
  imported: string[];
  /**
   * The secret names stored besides the accounts' (`integration:deepl`, ...).
   */
  secrets?: string[];
}
export interface ImportFailure {
  /**
   * Empty when the JSON had none.
   */
  accountId: string;
  email: string;
  error: string;
}
/**
 * `hello.credentials` and `engine.status.credentials`.
 */
export interface CredentialsStatus {
  /**
   * The accounts in `accounts.json`.
   */
  accounts: number;
  error?: string;
  /**
   * `keychain`, `secret-service` or `file`; absent when no store can be used, and then
   * `error` says why.
   */
  store?: string;
}
/**
 * Where a raw-feed client stopped: the `feed` of the engine it was connected to and the
 * last `delta.seq` it applied.
 */
export interface DeltaCursor {
  feed: string;
  seq: number;
}
/**
 * `delta` params: today's stdout delta object plus the account it came from and, on the
 * socket (`feature::DELTA_SEQ`), its place in the engine's
 * feed: one more than the delta before it, across accounts and classes.
 */
export interface DeltaParams {
  accountId: string;
  modelClass: string;
  modelJSONs: unknown[];
  seq?: number;
  /**
   * `persist`, `unpersist` or `metadata-expiration`.
   */
  type: string;
}
/**
 * `disconnect` params.
 */
export interface DisconnectParams {
  reason: string;
}
/**
 * `account.discover`'s account: the settings found for an address, before it has an id.
 */
export interface DiscoveredAccountJson {
  emailAddress: string;
  provider: string;
  /**
   * Servers, ports, usernames, security (never `none`), `*_allow_insecure_ssl` and
   * `container_folder`. An empty `imap_host`: nothing found.
   */
  settings: AccountSettingsJson;
}
/**
 * What a client's send hooks made of the body (`applyTransformsForSending`).
 */
export interface DraftBodies {
  /**
   * Recipient address to the body that recipient gets.
   */
  perRecipient?: {
    [k: string]: string;
  };
  /**
   * The body that goes out, and is saved to Sent.
   */
  self?: string;
}
/**
 * `draft.cancelSend`.
 */
export interface DraftCancelSendParams {
  accountId: string;
  headerMessageId: string;
}
export interface DraftCancelSendResult {
  /**
   * False when there was nothing to cancel: no pending send, or it has started.
   */
  cancelled: boolean;
}
/**
 * `draft.create`.
 */
export interface DraftCreateParams {
  /**
   * The account to write from: for `new`, `mailto` and `resurface`; a reply or forward is
   * written from the message's account. Without it, `core.sending.defaultAccountIdForSend`
   * or else the first account.
   */
  accountId?: string | null;
  /**
   * Files to attach, as `DraftUpdateParams::attach`. With `save: false` too: the engine
   * keeps the copies, and the draft's `files` carries them.
   */
  attach?: string[];
  /**
   * `reply`, `reply-all`: `prefer-existing` answers with the newest draft already replying
   * to the message (its recipients switched to the kind, then `fields` over it, `hMsgId`
   * aside), `prefer-existing-if-pristine` only when that draft is untouched. Without it a
   * new draft is made.
   */
  behavior?: string | null;
  /**
   * `resurface`: the body; the `resurfacing.snoozeBody` setting's by default.
   */
  body?: string | null;
  /**
   * Message JSON keys set over what the kind builds: `to`, `cc`, `bcc`, `from` (contact
   * lists), `subject`, `body`, `files`, `threadId`, `hMsgId`, `rthMsgId`, `fwdMsgId`,
   * `plaintext`, `pristine`; `aid` for `new`. A draft the engine saves (`save` not false)
   * is not pristine when these set any recipients, subject, body or files, unless they
   * set `pristine` themselves.
   */
  fields?: {
    [k: string]: unknown;
  } | null;
  /**
   * HTML or plaintext; `core.composing.html` by default.
   */
  html?: boolean | null;
  /**
   * One of `draft_kind`.
   */
  kind: string;
  /**
   * `mailto`: the link.
   */
  mailto?: string | null;
  /**
   * `reply`, `reply-all`, `forward`: the message (its id); `resurface`: the message it
   * answers, else the thread's newest.
   */
  messageId?: string | null;
  /**
   * False: answer with the draft without saving it (a client that saves it itself after
   * its own hooks). True by default.
   */
  save?: boolean | null;
  /**
   * A template's name (`feature::TEMPLATES`), applied after
   * `fields` as the desktop inserts one: its body replaces the text above the signature
   * and quote, and its subject is the draft's unless `fields` has a `subject` or the draft
   * is a reply or forward.
   */
  template?: string | null;
  /**
   * `resurface`: the thread.
   */
  threadId?: string | null;
  /**
   * The IANA zone for dates in quotes; the engine's zone by default.
   */
  tz?: string | null;
}
/**
 * `draft.delete`: queues a `DestroyDraftTask`. A send-later pending on the draft (an undo
 * window or a schedule) is cancelled in the same transaction, so the expiration executor
 * sends it wholly before or not at all; a draft whose `SendDraftTask` is queued can't be
 * deleted.
 */
export interface DraftDeleteParams {
  accountId: string;
  headerMessageId: string;
}
export interface DraftDeleteResult {
  /**
   * A pending send-later was cancelled with the draft.
   */
  cancelledSend: boolean;
  /**
   * The `DestroyDraftTask`.
   */
  taskId: string;
}
/**
 * One `draft.validate` finding.
 */
export interface DraftProblem {
  /**
   * `no-recipients`, `invalid-recipient`, `salutation-mismatch`, `unsendable-from`,
   * `blank-subject`, `attachment-mentioned`, `empty-template`, `template-variable`.
   */
  code: string;
  /**
   * `recipients` or `misc`.
   */
  group: string;
  /**
   * The desktop's English text.
   */
  message: string;
}
/**
 * `draft.send`.
 */
export interface DraftSendParams {
  accountId: string;
  /**
   * `send` (default) or `send-and-archive`.
   */
  after?: string | null;
  bodies?: DraftBodies | null;
  /**
   * The undo window. Without it (or 0) the draft is sent now; with it the engine writes
   * send-later metadata (`isUndoSend`) and sends when it is due, also after a restart.
   */
  delayMs?: number | null;
  headerMessageId: string;
}
export interface DraftSendResult {
  /**
   * Unix seconds, when the send waits out the undo window.
   */
  scheduledAt?: number;
  /**
   * The `SendDraftTask`, when sent now.
   */
  taskId?: string;
}
/**
 * `draft.update`: an edit to a stored draft, refused while it is being sent or has a
 * send-later (an undo window or a schedule) pending. Answers with the saved draft's Message
 * JSON and its `body`.
 */
export interface DraftUpdateParams {
  accountId: string;
  /**
   * Absolute paths of files on the engine's machine to attach, after `fields`. The engine
   * copies each into its files directory as a new File, named after the path and typed by
   * its extension; a file the draft already carries with the same name and bytes isn't
   * attached twice. Local clients only (`policy`).
   */
  attach?: string[];
  /**
   * Files to remove, after `attach`: a File id, or a filename that names one of the
   * draft's files. An upload's copy (a File with no `messageId`) is deleted with it.
   */
  detach?: string[];
  /**
   * Message JSON keys set over the stored draft, as `DraftCreateParams::fields` are over
   * a built one; `aid` and `hMsgId` stay. The draft is no longer pristine when these, or
   * `attach` or `detach`, write recipients, subject, body or files, unless they set
   * `pristine` themselves.
   */
  fields?: {
    [k: string]: unknown;
  } | null;
  headerMessageId: string;
}
/**
 * `draft.validate`.
 */
export interface DraftValidateParams {
  accountId: string;
  headerMessageId: string;
  /**
   * Recipients whose names aren't checked against the salutation (the desktop's "Send &
   * Ignore Warnings For This Email").
   */
  skipSalutationFor?: string[];
}
/**
 * Errors stop the send; warnings ask first.
 */
export interface DraftValidateResult {
  errors: DraftProblem[];
  warnings: DraftProblem[];
}
/**
 * The result of methods that return nothing: `{}`.
 */
export interface Empty {}
export interface EngineStatusResult {
  accounts: AccountStatus[];
  /**
   * As in `HelloResult::build`.
   */
  build?: string;
  /**
   * As in `HelloResult::capabilities`.
   */
  capabilities?: Capabilities;
  /**
   * As in `HelloResult::credentials`.
   */
  credentials?: CredentialsStatus;
  /**
   * As in `HelloResult::detached`.
   */
  detached?: boolean;
  engine: string;
  /**
   * As in `HelloResult::features`.
   */
  features?: string[];
  pid: number;
  protocol: number;
  /**
   * Seconds since the engine started.
   */
  uptimeSecs: number;
}
/**
 * One of `EventsExpandResult::conflicts`: a stored event overlapping the focused one.
 */
export interface EventConflict {
  calendarId: string;
  end: number;
  eventId: string;
  icsuid: string;
  start: number;
  title: string;
}
/**
 * A guest on an event.
 */
export interface EventGuest {
  email: string;
  name?: string;
  /**
   * `REQ-PARTICIPANT` by default.
   */
  role?: string;
}
/**
 * `EventsUpdateParams::patch`: the keys an edit may change. Schema only; the engine reads
 * the patch as a map.
 */
export interface EventPatch {
  /**
   * The whole list.
   */
  attendees?: EventGuest[];
  /**
   * Removes the occurrences the scope covers.
   */
  deleted?: boolean;
  description?: string;
  end?: number;
  isAllDay?: boolean;
  location?: string;
  /**
   * Null removes the rule (tern-pim reads the patch as JSON, so null and absent differ).
   */
  recurrenceRule?: string | null;
  start?: number;
  summary?: string;
  timezone?: string;
  transparency?: string;
}
/**
 * `upcoming` and `first` of `EventsExpandResult`: an invitation's next occurrence, or its
 * own times.
 */
export interface EventTimes {
  end: number;
  isAllDay: boolean;
  recurrenceId?: number;
  start: number;
}
/**
 * One calendar write an edit made: `update` and `destroy` name the row, `create` the
 * calendar.
 */
export interface EventWrite {
  calendarId?: string;
  eventId?: string;
  ics?: string;
  /**
   * `create`, `update` or `destroy`.
   */
  kind: string;
  /**
   * The task queued for it.
   */
  taskId?: string;
}
/**
 * `events.changed`: the subscription's occurrences, whole, after an event or calendar of
 * its accounts changed them.
 */
export interface EventsChangedParams {
  occurrences: unknown[];
  subId: string;
  truncated?: boolean;
}
export interface EventsConflicts {
  /**
   * The invitation being answered, left out of its own conflicts.
   */
  excludeIcsuid?: string;
}
/**
 * A counter-proposal's time.
 */
export interface EventsCounter {
  end: number;
  start: number;
}
/**
 * `events.create`: a new event on a calendar, organised by the account when it has guests.
 */
export interface EventsCreateParams {
  accountId: string;
  attendees?: EventGuest[];
  calendarId: string;
  description?: string | null;
  end: number;
  isAllDay?: boolean;
  location?: string | null;
  /**
   * False: answer with the writes without queueing them (a client that queues them
   * itself, for undo). True by default.
   */
  queue?: boolean | null;
  /**
   * An RRULE value (`FREQ=WEEKLY;BYDAY=MO`).
   */
  recurrenceRule?: string | null;
  start: number;
  summary: string;
  /**
   * The zone the times are written in; UTC when absent.
   */
  timezone?: string | null;
  /**
   * `OPAQUE` or `TRANSPARENT`.
   */
  transparency?: string | null;
  tz?: string | null;
}
/**
 * `events.expand`: the occurrences of stored events that overlap `[start, end]`, or of an
 * ICS that is not stored (`ics`, an emailed invitation). Rows are chosen by `eventIds`,
 * else `calendarIds`, else every calendar of `accountIds` (every account by default); an
 * event id brings every row of its UID.
 */
export interface EventsExpandParams {
  accountIds?: string[] | null;
  calendarIds?: string[] | null;
  /**
   * Also answer `conflicts`: the timed occurrences of the chosen rows that overlap
   * `[start, end)` and that the account attends.
   */
  conflicts?: EventsConflicts | null;
  end: number;
  eventIds?: string[] | null;
  /**
   * With `eventIds`: also answer `focused`, the occurrence to show for an event opened
   * from outside the calendar. With `ics`: also answer `upcoming` and `first`.
   */
  focus?: EventsFocus | null;
  /**
   * An ICS to expand instead of stored rows, read as the account `accountIds` names
   * first would have it.
   */
  ics?: string | null;
  start: number;
  tz?: string | null;
}
export interface EventsFocus {
  /**
   * The time to look forward from.
   */
  now: number;
  /**
   * The occurrence to focus, by the slot it fills.
   */
  recurrenceIdStart?: number;
}
/**
 * `events.expand`'s answer. An occurrence has the desktop's `EventOccurrence` keys
 * (`id`, `eventId`, `accountId`, `calendarId`, `title`, `start`, `end`, `startDate`,
 * `endDate` as days since 1970-01-01, `isAllDay`, `isRecurring`, `isException`,
 * `recurrenceIdStart`, `organizer`, `attendees`, `isMine`, `myStatus`, ...).
 */
export interface EventsExpandResult {
  conflicts?: EventConflict[];
  /**
   * With `ics` and `focus`: the invitation's own times, from its first VEVENT, in the
   * same shape; `recurrenceId` when it is about one occurrence of a series.
   */
  first?: EventTimes;
  focused?: unknown;
  occurrences: unknown[];
  /**
   * More occurrences overlap the window than one reply carries
   * (`EVENTS_EXPAND_MAX_OCCURRENCES`); ask for a narrower one.
   */
  truncated?: boolean;
  /**
   * `{start, end, isAllDay, recurrenceId?}`; null once a series is over.
   */
  upcoming?: EventTimes;
}
/**
 * `events.rsvp`: our answer to an invitation, on our copy (`eventId`) or on an emailed
 * one (`ics`, with `messageId`), written to our copy and emailed to the organizer as a
 * REPLY; or, with `counter`, a COUNTER proposing another time, with nothing written.
 */
export interface EventsRsvpParams {
  accountId: string;
  /**
   * With `ics` and no stored copy: the calendar to add it to when attending; the
   * account's first writable calendar by default.
   */
  calendarId?: string | null;
  comment?: string | null;
  counter?: EventsCounter | null;
  eventId?: string | null;
  ics?: string | null;
  /**
   * The message the invitation came in; the REPLY answers it.
   */
  messageId?: string | null;
  /**
   * The one occurrence of a series to answer, by the slot it fills.
   */
  occurrenceStart?: number | null;
  /**
   * False: don't email the organizer. True by default.
   */
  reply?: boolean | null;
  /**
   * Set on an answer made again on a copy that changed under the first; not retried a
   * second time.
   */
  retried?: boolean | null;
  /**
   * `ACCEPTED`, `TENTATIVE` or `DECLINED`.
   */
  status: string;
  /**
   * The subject; `Accepted: <title>` (or `New time proposed: <title>`) by default.
   */
  subject?: string | null;
  tz?: string | null;
  /**
   * False: don't write our copy. True by default.
   */
  write?: boolean | null;
}
export interface EventsRsvpResult {
  /**
   * Our address on the guest list.
   */
  email: string;
  /**
   * The `EventRSVPTask`.
   */
  replyTaskId?: string;
  write?: EventWrite;
}
/**
 * `events.subscribe`: the occurrences of the stored events in `[start, end)`, as
 * `events.expand` gives them, kept current. `key` as in `query.subscribe`.
 */
export interface EventsSubscribeParams {
  accountIds?: string[] | null;
  calendarIds?: string[] | null;
  end: number;
  key?: string | null;
  start: number;
  tz?: string | null;
}
/**
 * `events.subscribe`'s result.
 */
export interface EventsSubscribeResult {
  occurrences: unknown[];
  subId: string;
  /**
   * As in `EventsExpandResult::truncated`.
   */
  truncated?: boolean;
}
/**
 * `events.update`: `patch` on one occurrence, that one and the ones after, or the whole
 * series. On an event that does not repeat every scope is `all`.
 */
export interface EventsUpdateParams {
  accountId: string;
  eventId: string;
  /**
   * For `this` and `following`: the occurrence, by the slot it fills (its
   * `recurrenceIdStart`, else its start). Not needed for an exception row.
   */
  occurrenceStart?: number | null;
  /**
   * `start`, `end`, `isAllDay`, `timezone`, `summary`, `location`, `description`,
   * `attendees` (`[{email, name?, role?}]`, the whole list), `transparency`,
   * `recurrenceRule` (null removes it), `deleted: true` (removes the occurrences the
   * scope covers).
   */
  patch: EventPatch;
  queue?: boolean | null;
  /**
   * `this`, `following` or `all` (default).
   */
  scope?: EventScope | null;
  tz?: string | null;
}
/**
 * `events.create` and `events.update`.
 */
export interface EventsWriteResult {
  /**
   * `events.create`: the new event's UID.
   */
  icsuid?: string;
  /**
   * `events.update`: the account organizes the event and it has other guests, so the
   * calendar server mails them the change or cancellation (RFC 6638 section 3.2.1).
   */
  notifiesGuests?: boolean;
  writes: EventWrite[];
}
export interface FilesGetParams {
  accountId: string;
  fileId: string;
}
/**
 * `files.get`: the file as it is now. The engine asks for the message's body when the file
 * isn't on disk and answers once it is there, or with `downloaded: false` (and no `path`)
 * when it doesn't appear in time.
 */
export interface FilesGetResult {
  contentType: string;
  downloaded: boolean;
  /**
   * The File's `filename`.
   */
  filename?: string;
  path?: string;
  /**
   * Bytes on disk when downloaded, else the size the File records.
   */
  size: number;
}
/**
 * `files.put`: one of `data` (base64) or `path` (a file the engine copies; the caller may
 * delete it once answered). Refused on relayed connections even with the admin scope, so a
 * remote device can't read the engine's file system.
 */
export interface FilesPutParams {
  accountId: string;
  contentType?: string | null;
  data?: string | null;
  name: string;
  path?: string | null;
}
/**
 * `files.put`'s answer: the new File's id, and the File JSON a draft's `files` carries.
 */
export interface FilesPutResult {
  file: FileJson;
  fileId: string;
}
export interface FilesReadParams {
  accountId: string;
  fileId: string;
  /**
   * At most `FILES_READ_MAX_BYTES`; the default.
   */
  len?: number | null;
  offset?: number;
}
/**
 * `files.read`: one chunk of a downloaded file, base64 (standard alphabet, padded).
 */
export interface FilesReadResult {
  data: string;
  /**
   * The chunk ends at the end of the file.
   */
  eof: boolean;
  offset: number;
  /**
   * The file's whole size.
   */
  size: number;
}
/**
 * `folders.list`: the account's folders by path. `counts` is keyed by folder id and
 * leaves out folders without a `ThreadCounts` row.
 */
export interface FoldersListResult {
  counts: {
    [k: string]: CategoryCounts;
  };
  folders: FolderJson[];
}
export interface HelloParams {
  /**
   * Free-form, e.g. `tern-cli/0.1.0`; logged by the engine.
   */
  client?: string;
  /**
   * The relayed device, for the engine's log.
   */
  device?: RelayDevice | null;
  protocol: number;
  /**
   * Set by `tern-gateway` for a connection it relays for a remote device. The engine
   * then applies `policy` to it. The first `hello` fixes this, `scopes`
   * and `device` for the connection.
   */
  relayed?: boolean;
  /**
   * The relayed device's scopes, e.g. `policy::SCOPE_ADMIN`.
   */
  scopes?: string[];
}
export interface RelayDevice {
  id: string;
  name: string;
}
export interface HelloResult {
  accounts: AccountInfo[];
  /**
   * The binary's identity (`feature::ACCOUNTS`): `sha256:`
   * and the hex SHA-256 of the executable the engine was started from, so a client can
   * tell an engine from another build from the one it bundles.
   */
  build?: string;
  /**
   * What the engine can do for the client now (`feature::CAPABILITIES`);
   * `capabilities.changed` carries later values.
   */
  capabilities?: Capabilities;
  /**
   * The secret store and how many accounts are registered (`feature::ACCOUNTS`).
   */
  credentials?: CredentialsStatus;
  /**
   * `serve --detached`: no parent channel; the parent methods are taken on the socket.
   */
  detached?: boolean;
  /**
   * The engine's version.
   */
  engine: string;
  /**
   * What this engine offers beyond protocol v1's base methods, from
   * `feature`. Clients gate behaviour on these, never on `engine`. Empty
   * from an engine that predates them.
   */
  features?: string[];
  pid: number;
  protocol: number;
}
/**
 * An integration provider with its settings, key included (tern-integrations
 * `ProviderConfig`), as `config` in the `integrations.*` params.
 */
export interface IntegrationConfig {
  apiKey?: string;
  /**
   * `libretranslate`, `deepl`, `languagetool` or `peopledatalabs`.
   */
  provider: string;
  url?: string;
  username?: string;
}
/**
 * The `integrations.*` params: the provider's `config` and the call's own fields (`text`,
 * `target`, `email`, ...). Without `config` the engine uses its settings
 * (`feature::ACCOUNT_SETUP`).
 */
export interface IntegrationParams {
  config?: IntegrationConfig | null;
  [k: string]: unknown;
}
/**
 * `integrations.test`'s answer (tern-integrations `TestReport`).
 */
export interface IntegrationTestReport {
  /**
   * The host that answered.
   */
  host: string;
  usage?: IntegrationUsage;
}
/**
 * Characters used in the billing period (DeepL).
 */
export interface IntegrationUsage {
  limit?: number;
  used: number;
}
/**
 * `labels.list`, shaped like `FoldersListResult`.
 */
export interface LabelsListResult {
  counts: {
    [k: string]: CategoryCounts;
  };
  labels: LabelJson[];
}
/**
 * `engine.logs.tail` params. Without `accountId`, the engine's own log (`engine.log`).
 */
export interface LogsTailParams {
  accountId?: string | null;
  /**
   * At most `LOGS_TAIL_MAX_BYTES`; `LOGS_TAIL_DEFAULT_BYTES` when absent.
   */
  bytes?: number | null;
}
/**
 * `engine.logs.tail` result. `text` is the last `bytes` of the current log file, decoded
 * lossily; empty when the file does not exist.
 */
export interface LogsTailResult {
  text: string;
  /**
   * The file is longer than `text`.
   */
  truncated: boolean;
}
/**
 * `mail.new` params: a message that arrived unread in the inbox after the folder's first
 * headers sync. Sent once per message to the connections subscribed to its account, never
 * held for a later subscriber; a client that sees a `messageId` twice drops the repeat.
 */
export interface MailNewParams {
  accountId: string;
  messageId: string;
  threadId: string;
}
/**
 * `messages.get`.
 */
export interface MessagesGetParams {
  accountId: string;
  /**
   * Include the body from `MessageBody`.
   */
  body?: boolean;
  id: string;
}
/**
 * `body` is null unless asked for and fetched (`bodies.fetch` fetches it). `files` is the
 * message's `File` JSON.
 */
export interface MessagesGetResult {
  body?: string | null;
  files: FileJson[];
  message: MessageJson;
}
/**
 * `messages.unsubscribe`: act on the message's `List-Unsubscribe` header.
 */
export interface MessagesUnsubscribeParams {
  accountId: string;
  messageId: string;
  /**
   * One of `unsubscribe_method`; without it the engine picks one-click when the message
   * offers it, else mailto, else web. A method the header doesn't offer is `NOT_FOUND`.
   */
  method?: string | null;
}
export interface MessagesUnsubscribeResult {
  /**
   * Why `ok` is false.
   */
  error?: string;
  /**
   * The `unsubscribe_method` used.
   */
  method: string;
  /**
   * one-click: the server answered 2xx. mailto: the message is queued. web: always true,
   * and the client opens `url`.
   */
  ok: boolean;
  /**
   * mailto: the `SendDraftTask` that sends the message.
   */
  taskId?: string;
  /**
   * The URI acted on.
   */
  url?: string;
}
/**
 * `metadata.list`: the models carrying `pluginId`'s metadata (a non-empty value), newest
 * first by `date` (Message) or `lastMessageTimestamp` (Thread), then id. `objectType` is
 * `Message` (the default) or `Thread`; `draft` only applies to messages. `expiresBefore`
 * keeps rows whose expiration is set and earlier; `hasExpiration` rows with (true) or
 * without (false) one. `after` and `before` bound the model's date, exclusive, in epoch
 * seconds. `offset` skips rows, for paging past `READ_MAX_LIMIT`.
 */
export interface MetadataListParams {
  accounts?: string[] | null;
  after?: number | null;
  before?: number | null;
  draft?: boolean | null;
  expiresBefore?: number | null;
  hasExpiration?: boolean | null;
  limit?: number | null;
  objectType?: string | null;
  offset?: number | null;
  pluginId: string;
}
export interface MetadataListResult {
  models: MetadataModelJson[];
}
/**
 * The JSON of each `modelClass` a delta carries.
 */
export interface ModelJsonByClass {
  Calendar: CalendarJson;
  Contact: ContactJson;
  ContactBook: ContactBookJson;
  ContactGroup: ContactGroupJson;
  Event: EventJson;
  File: FileJson;
  Folder: FolderJson;
  Label: LabelJson;
  Message: MessageJson;
  ProcessIdentityRefreshNeeded: never;
  ProcessState: ProcessStateJson;
  Task: TaskJson;
  Thread: ThreadJson;
  ThreadCounts: ThreadCountsJson;
}
/**
 * A `ProcessState` delta's model: not stored, the account's online state.
 */
export interface ProcessStateJson {
  accountId: string;
  connectionError: boolean;
  /**
   * The account id.
   */
  id?: string;
}
/**
 * A `ThreadCounts` delta's model: `persist` when a count changes or a category is added,
 * `unpersist` (no counts) when it is removed (`feature::COUNTS`).
 */
export interface ThreadCountsJson {
  categoryId: string;
  id: string;
  total?: number;
  unread?: number;
}
/**
 * `account.oauth.begin` params.
 */
export interface OAuthBeginParams {
  /**
   * Reconnect: the registered account the sign-in renews. `complete` refuses a sign-in
   * to another mailbox and keeps the account's id and settings.
   */
  accountId?: string | null;
  /**
   * The client's default `container_folder` for a new account.
   */
  containerFolder?: string | null;
  /**
   * `gmail`, `office365` or `outlook`.
   */
  provider: string;
  /**
   * The loopback port the client's redirect receiver listens on (default 12141).
   */
  redirectPort?: number | null;
  /**
   * Office 365: the shared mailbox to open with the signed-in user's grant.
   */
  sharedMailbox?: string | null;
}
/**
 * `account.oauth.begin` result. The client opens `authUrl` in a browser and passes the
 * `code` and `state` the provider sends to `redirectUri` to `account.oauth.complete`.
 */
export interface OAuthBeginResult {
  authUrl: string;
  redirectUri: string;
  sessionId: string;
  /**
   * The `state` in `authUrl`, so the receiver can drop a redirect meant for another
   * sign-in before it reaches the engine. The engine checks it again.
   */
  state: string;
}
/**
 * `account.oauth.complete` params. A session is used once, and expires 15 minutes after
 * `begin`.
 */
export interface OAuthCompleteParams {
  code: string;
  sessionId: string;
  state: string;
  /**
   * Test the account before storing it (default true), as `account.add`'s `test`.
   */
  test?: boolean | null;
}
/**
 * `account.oauth.complete` result: the account the engine tested, stored and started.
 */
export interface OAuthCompleteResult {
  /**
   * Its JSON without secrets.
   */
  account: AccountJson;
  accountId: string;
  error?: SyncErrorJson;
  state: 'starting' | 'running' | 'stopped' | 'removed';
  /**
   * The account and its secrets are in the engine's registry and secret store, so the
   * client need not keep them (`feature::ACCOUNTS`). False
   * when no secret store could take them: the engine runs the account until it exits.
   */
  stored?: boolean;
}
/**
 * The `data` of `code::AI_FAILED` and
 * `code::INTEGRATION_FAILED`: tern-ai `AiError` and
 * tern-integrations `IntegrationError` as they serialise.
 */
export interface ProviderErrorJson {
  key: string;
  message: string;
  /**
   * Seconds.
   */
  retryAfter?: number;
  retryable: boolean;
  /**
   * The provider's HTTP status.
   */
  status?: number;
}
/**
 * `query.changed`: what changed in a subscription's window, at most once per 50 ms. Either
 * `ops`, or `reset` with the whole window when the steps would outnumber its rows.
 * `version` is one more than the last one the client saw for the subscription.
 */
export interface QueryChangedParams {
  ops?: LiveOp[];
  reset?: AnyModelJson[];
  subId: string;
  total: number;
  version: number;
}
export interface QueryOrder {
  attr: string;
  /**
   * `"asc"` or `"desc"`.
   */
  dir: 'asc' | 'desc';
}
/**
 * `query`: the JSON form of the desktop's `ModelQuery`
 * (`Query.toJSON()`). `where` is a matcher (`{attr, op, value}`, `{and|or|not: [..]}`) or a
 * list of them, all of which must match. `accounts: None` reads every account. An empty
 * `order` is the class's natural order. `cursor` pages by keyset: `""` for the first page,
 * then the last page's `nextCursor`. No limit is applied when `limit` is absent.
 */
export interface QueryParams {
  accounts?: string[] | null;
  class: QueryClassName;
  count?: boolean;
  cursor?: string | null;
  distinct?: boolean;
  idsOnly?: boolean;
  include?: string[];
  limit?: number | null;
  offset?: number | null;
  order?: QueryOrder[];
  search?: QuerySearch | null;
  where?: QueryWhere | null;
}
/**
 * `query.search`. `grammar: false` is the plain FTS prefix match of `.search(text)`;
 * `grammar: true` (feature `search-grammar`, Thread only) is the search grammar, with date
 * terms read in `tz` (IANA, UTC when absent) and counted back from `now` (unix seconds).
 */
export interface QuerySearch {
  grammar?: boolean;
  now?: number;
  text: string;
  tz?: string;
}
/**
 * `query`'s result: `rows` always (empty for `idsOnly` and `count`), `ids` for `idsOnly`,
 * `count` for `count`, and `nextCursor` for a cursor query (`null` after the last page).
 */
export interface QueryResult {
  count?: number;
  ids?: string[];
  /**
   * `null` after the last page.
   */
  nextCursor?: string | null;
  /**
   * The class's model JSON (with `include`'s joined data).
   */
  rows: AnyModelJson[];
}
/**
 * `query.setWindow`: moves the subscription's window.
 */
export interface QuerySetWindowParams {
  subId: string;
  window: QueryWindow;
}
/**
 * Which rows of a live query the client holds: `limit` rows from `offset` in the query's
 * order. No `limit` holds every row from `offset`; a relayed connection's window stops at
 * `READ_MAX_LIMIT` rows.
 */
export interface QueryWindow {
  limit?: number;
  offset?: number;
}
/**
 * `query.subscribe`. `query` is a `query` request without `limit`, `offset`, `cursor`,
 * `idsOnly` or `count`: the window decides which rows come back. `window` absent holds
 * every row. `key` names the subscription on this connection: a later `query.subscribe`
 * with the same key ends the earlier one, as `query.unsubscribe` would.
 */
export interface QuerySubscribeParams {
  key?: string | null;
  query: QueryParams;
  window?: QueryWindow | null;
}
/**
 * `query.subscribe`'s result: the window's rows, how many rows the whole query has, and
 * the version the next `query.changed` builds on.
 */
export interface QuerySubscribeResult {
  rows: AnyModelJson[];
  subId: string;
  total: number;
  version: number;
}
/**
 * `query.setWindow`'s result: the new window's rows, replacing the ones the client holds.
 */
export interface QueryWindowResult {
  rows: AnyModelJson[];
  total: number;
  version: number;
}
/**
 * `engine.resetDatabase` params, all optional. `keepMetadata` (default true) carries the
 * plugin metadata into the new database; false when the last account was removed.
 */
export interface ResetDatabaseParams {
  keepMetadata?: boolean;
}
/**
 * `engine.resetDatabase` result: the accounts it stopped and forgot.
 */
export interface ResetDatabaseResult {
  removed: string[];
}
/**
 * Each method's params and result.
 */
export interface RpcMethods {
  /**
   * `serve` mode: register an account, store its secrets and start syncing it
   * (`AccountAddParams`). On the parent's stdin, and on the
   * socket of an engine with `feature::ACCOUNT_SETUP`
   * (local-only class, `policy`).
   */
  'account.add': {
    params: AccountAddParams;
    result: AccountAddResult;
  };
  /**
   * IMAP and SMTP settings for an address (`AccountDiscoverParams`).
   * Open to relayed connections; answered out of order (MX and autoconfig lookups).
   */
  'account.discover': {
    params: AccountDiscoverParams;
    result: AccountDiscoverResult;
  };
  /**
   * Every account the engine holds, with its state.
   */
  'account.list': {
    params: undefined;
    result: AccountListResult;
  };
  /**
   * Starts an OAuth sign-in: the engine keeps the PKCE verifier, `state` and client
   * secret (`OAuthBeginParams`).
   */
  'account.oauth.begin': {
    params: OAuthBeginParams;
    result: OAuthBeginResult;
  };
  /**
   * Ends it with the redirect's `code` and `state`: exchange, profile, discovery, test,
   * then the account is stored and started (`OAuthCompleteParams`).
   * Answered out of order.
   */
  'account.oauth.complete': {
    params: OAuthCompleteParams;
    result: OAuthCompleteResult;
  };
  /**
   * `serve` mode: stop an account, unregister it and delete its secrets, with `revoke`
   * also its OAuth grant and with `deleteData` its rows
   * (`AccountRemoveParams`). Without `deleteData` the rows
   * stay. Where `account.add` is taken.
   */
  'account.remove': {
    params: AccountRemoveParams;
    result: AccountRemoveResult;
  };
  /**
   * The order of `account.list` (`AccountReorderParams`).
   */
  'account.reorder': {
    params: AccountReorderParams;
    result: Empty;
  };
  /**
   * `serve` mode: stop the account if the engine holds it, delete its rows from the
   * database (`--mode reset`) and start it again from the registry to resync (a client
   * that adds it again gets ACCOUNT_EXISTS). Where `account.add` is taken; answered out of
   * order.
   */
  'account.reset': {
    params: AccountResetParams;
    result: Empty;
  };
  /**
   * New credentials for a registered account (`AccountSetSecretsParams`).
   */
  'account.setSecrets': {
    params: AccountSetSecretsParams;
    result: AccountAddResult;
  };
  /**
   * `serve` mode: check an account's IMAP and SMTP settings, as `--mode test` does
   * (`AccountTestParams`). The result is that mode's JSON
   * object, with any rotated refresh token in its `account` on stdin only. Where
   * `account.add` is taken; answered out of order.
   */
  'account.test': {
    params: AccountTestParams;
    result: AccountTestResult;
  };
  /**
   * Changes an account's JSON (`AccountUpdateParams`).
   * Relayed devices without the admin scope may change only
   * `ACCOUNT_DISPLAY_FIELDS`.
   */
  'account.update': {
    params: AccountUpdateParams;
    result: AccountUpdateResult;
  };
  /**
   * Answered at once, in order: stops the calling connection's stream `streamId`.
   */
  'ai.cancel': {
    params: AiCancelParams;
    result: Empty;
  };
  'ai.draftReply': {
    params: AiDraftReplyParams;
    result: AiCompletion;
  };
  /**
   * The `ai.*` methods (`feature::AI`), params and results
   * in `crate::AiSettings`. Until the engine keeps the AI
   * settings and keys, local clients send the provider `config`, key included, and the
   * `settings` with each call; a relayed connection may send neither
   * (`policy`). Answered out of order on the engine's AI workers (T21);
   * a failed model call is `code::AI_FAILED`.
   */
  'ai.models': {
    params: AiProviderParams;
    result: AiModelInfo[];
  };
  'ai.suggestReplies': {
    params: AiSuggestRepliesParams;
    result: AiSuggestion[];
  };
  'ai.summarize': {
    params: AiSummarizeParams;
    result: AiCompletion;
  };
  'ai.test': {
    params: AiProviderParams;
    result: AiTestReport;
  };
  'ai.triage.list': {
    params: AiTriageListParams;
    result: AiTriageListResult;
  };
  /**
   * Inbox triage; not served yet.
   */
  'ai.triage.run': {
    params: AiTriageRunParams;
    result: AiTriageRunResult;
  };
  /**
   * Token counts per day, provider and feature since the engine started.
   */
  'ai.usage': {
    params: AiUsageParams;
    result: AiUsageRow[];
  };
  'bodies.fetch': {
    params: BodiesFetchParams;
    result: Empty;
  };
  'calendar.sync': {
    params: CalendarSyncParams;
    result: CalendarSyncResult;
  };
  /**
   * A new contact, params `ContactsCreateParams`.
   */
  'contacts.create': {
    params: ContactsCreateParams;
    result: ContactDetails;
  };
  /**
   * Contacts as a .vcf file, params `ContactsExportParams`.
   */
  'contacts.export': {
    params: ContactsExportParams;
    result: ContactsExportResult;
  };
  /**
   * A contact's details (`feature::CONTACT_EDITING`),
   * params `ContactsGetParams`.
   */
  'contacts.get': {
    params: ContactsGetParams;
    result: ContactDetails;
  };
  /**
   * Contacts from a .vcf file, params `ContactsImportParams`.
   */
  'contacts.import': {
    params: ContactsImportParams;
    result: ContactsImportResult;
  };
  'contacts.search': {
    params: ContactsSearchParams;
    result: ContactsSearchResult;
  };
  /**
   * An edit to a contact, params `ContactsUpdateParams`.
   */
  'contacts.update': {
    params: ContactsUpdateParams;
    result: ContactDetails;
  };
  /**
   * Every `ThreadCounts` row of the accounts (`feature::COUNTS`).
   */
  'counts.get': {
    params: CountsGetParams;
    result: CountsGetResult;
  };
  /**
   * `serve` mode, the parent's stdin only: register accounts and store secrets a client
   * held until now, params
   * `CredentialsImportParams`.
   */
  'credentials.import': {
    params: CredentialsImportParams;
    result: CredentialsImportResult;
  };
  /**
   * Cancels a send waiting out its undo window, params
   * `DraftCancelSendParams`.
   */
  'draft.cancelSend': {
    params: DraftCancelSendParams;
    result: DraftCancelSendResult;
  };
  /**
   * Builds a draft as the desktop does and saves it
   * (`feature::DRAFT_PIPELINE`), params
   * `DraftCreateParams`; answers with its Message JSON.
   */
  'draft.create': {
    params: DraftCreateParams;
    result: MessageJson;
  };
  /**
   * Deletes a draft, params `DraftDeleteParams`.
   */
  'draft.delete': {
    params: DraftDeleteParams;
    result: DraftDeleteResult;
  };
  /**
   * Sends a draft now or after an undo window, params
   * `DraftSendParams`.
   */
  'draft.send': {
    params: DraftSendParams;
    result: DraftSendResult;
  };
  /**
   * An edit to a stored draft (`feature::DRAFT_EDITING`),
   * params `DraftUpdateParams`; answers with its Message JSON.
   */
  'draft.update': {
    params: DraftUpdateParams;
    result: MessageJson;
  };
  /**
   * The checks before a send, params `DraftValidateParams`.
   */
  'draft.validate': {
    params: DraftValidateParams;
    result: DraftValidateResult;
  };
  /**
   * The end of an account's log, or of the engine's own log.
   */
  'engine.logs.tail': {
    params: LogsTailParams;
    result: LogsTailResult;
  };
  /**
   * `serve` mode, the parent's stdin only: stop and forget every account, delete the
   * database and create an empty one. The client adds the accounts again.
   */
  'engine.resetDatabase': {
    params: ResetDatabaseParams;
    result: ResetDatabaseResult;
  };
  /**
   * `serve` mode, the parent's stdin only: verbose protocol logging until a time, for
   * every account; running accounts restart to pick it up.
   */
  'engine.setLogLevel': {
    params: SetLogLevelParams;
    result: SetLogLevelResult;
  };
  /**
   * `serve` mode, the parent's stdin only (the socket too on a `--detached` engine): stop
   * every account, remove the socket files and exit 0.
   */
  'engine.shutdown': {
    params: undefined;
    result: Empty;
  };
  'engine.status': {
    params: undefined;
    result: EngineStatusResult;
  };
  /**
   * A new event, params `EventsCreateParams`.
   */
  'events.create': {
    params: EventsCreateParams;
    result: EventsWriteResult;
  };
  /**
   * Occurrences of stored events or of an ICS
   * (`feature::CALENDAR_EDITING`), params
   * `EventsExpandParams`.
   */
  'events.expand': {
    params: EventsExpandParams;
    result: EventsExpandResult;
  };
  /**
   * An answer to an invitation or a counter-proposal, params
   * `EventsRsvpParams`.
   */
  'events.rsvp': {
    params: EventsRsvpParams;
    result: EventsRsvpResult;
  };
  /**
   * Occurrences kept current (`feature::EVENTS_SUBSCRIBE`),
   * params `EventsSubscribeParams`; then `events.changed`.
   * Answered as `query.subscribe` is.
   */
  'events.subscribe': {
    params: EventsSubscribeParams;
    result: EventsSubscribeResult;
  };
  /**
   * Ends an `events.subscribe`, params `SubIdParams`.
   */
  'events.unsubscribe': {
    params: SubIdParams;
    result: Empty;
  };
  /**
   * An edit to an event or some of its occurrences, params
   * `EventsUpdateParams`.
   */
  'events.update': {
    params: EventsUpdateParams;
    result: EventsWriteResult;
  };
  /**
   * A File by id, downloaded first when missing (`feature::FILES`),
   * params `FilesGetParams`. Local clients only; answered out of
   * order while it waits for the download.
   */
  'files.get': {
    params: FilesGetParams;
    result: FilesGetResult;
  };
  /**
   * Stores bytes as a new File for a draft, params `FilesPutParams`.
   */
  'files.put': {
    params: FilesPutParams;
    result: FilesPutResult;
  };
  /**
   * A chunk of a downloaded File, params `FilesReadParams`.
   */
  'files.read': {
    params: FilesReadParams;
    result: FilesReadResult;
  };
  'folders.list': {
    params: CategoriesListParams;
    result: FoldersListResult;
  };
  hello: {
    params: HelloParams;
    result: HelloResult;
  };
  'integrations.detect': {
    params: IntegrationParams;
    result: unknown;
  };
  'integrations.enrich': {
    params: IntegrationParams;
    result: unknown;
  };
  'integrations.grammar': {
    params: IntegrationParams;
    result: unknown;
  };
  'integrations.languages': {
    params: IntegrationParams;
    result: unknown;
  };
  'integrations.test': {
    params: IntegrationParams;
    result: IntegrationTestReport;
  };
  /**
   * The `integrations.*` methods (`feature::INTEGRATIONS`).
   * Each carries the
   * provider's `config`, key included, until the engine owns the credentials. Local
   * clients only, answered out of order on the engine's integrations workers (T21); a
   * provider failure is `code::INTEGRATION_FAILED`.
   */
  'integrations.translate': {
    params: IntegrationParams;
    result: unknown;
  };
  'labels.list': {
    params: CategoriesListParams;
    result: LabelsListResult;
  };
  'messages.get': {
    params: MessagesGetParams;
    result: MessagesGetResult;
  };
  /**
   * Unsubscribes from the list a message came from
   * (`feature::UNSUBSCRIBE`), params
   * `MessagesUnsubscribeParams`. Socket only; answered
   * out of order while a one-click POST runs.
   */
  'messages.unsubscribe': {
    params: MessagesUnsubscribeParams;
    result: MessagesUnsubscribeResult;
  };
  /**
   * Messages or threads carrying a plugin's metadata
   * (`feature::METADATA_LIST`).
   */
  'metadata.list': {
    params: MetadataListParams;
    result: MetadataListResult;
  };
  /**
   * A ModelQuery over the stored models (`feature::QUERY`).
   */
  query: {
    params: QueryParams;
    result: QueryResult;
  };
  /**
   * Moves a live query's window, params `QuerySetWindowParams`.
   */
  'query.setWindow': {
    params: QuerySetWindowParams;
    result: QueryWindowResult;
  };
  /**
   * A live query (`feature::LIVE_QUERIES`), params
   * `QuerySubscribeParams`: the window's rows now, then
   * `query.changed` on this connection whenever they change. Answered out of order with
   * the connection's other requests, in order with its `query.changed`.
   */
  'query.subscribe': {
    params: QuerySubscribeParams;
    result: QuerySubscribeResult;
  };
  /**
   * Ends a live query, params `SubIdParams`; no `query.changed`
   * for it follows the answer. Closing the connection ends all of them.
   */
  'query.unsubscribe': {
    params: SubIdParams;
    result: Empty;
  };
  'rules.cancel': {
    params: RulesCancelParams;
    result: RulesCancelResult;
  };
  /**
   * Mail rules (`feature::RULES`): params and results in
   * `crate::RulesReprocessParams` and `crate::RulesCancelParams`. The rules themselves
   * are the `mailRules` setting.
   */
  'rules.reprocess': {
    params: RulesReprocessParams;
    result: RulesReprocessResult;
  };
  search: {
    params: SearchParams;
    result: SearchResult;
  };
  'secrets.delete': {
    params: SecretsDeleteParams;
    result: Empty;
  };
  /**
   * Which of the named secrets are stored (`SecretsHasParams`).
   */
  'secrets.has': {
    params: SecretsHasParams;
    result: {
      [k: string]: boolean;
    };
  };
  /**
   * Stores a secret that isn't an account's (`SecretsSetParams`).
   * There is no `secrets.get`.
   */
  'secrets.set': {
    params: SecretsSetParams;
    result: Empty;
  };
  /**
   * Forgets it and its token; accounts sync metadata locally.
   */
  'server.clear': {
    params: undefined;
    result: ServerSetResult;
  };
  /**
   * The Tern server the accounts sync metadata with (`ServerSetParams`).
   */
  'server.set': {
    params: ServerSetParams;
    result: ServerSetResult;
  };
  /**
   * The stored engine-owned settings (`feature::SETTINGS`),
   * params `SettingsGetParams`, result
   * `SettingsValues`.
   */
  'settings.get': {
    params: SettingsGetParams;
    result: SettingsValues;
  };
  /**
   * Stores settings, params `SettingsValues`, result `{}`.
   */
  'settings.set': {
    params: SettingsValues;
    result: Empty;
  };
  subscribe: {
    params: SubscribeParams;
    result: SubscribeResult;
  };
  'sync.wake': {
    params: SyncWakeParams;
    result: Empty;
  };
  'task.cancel': {
    params: TaskCancelParams;
    result: Empty;
  };
  'task.queue': {
    params: TaskQueueParams;
    result: TaskJson;
  };
  'tasks.get': {
    params: TasksGetParams;
    result: TasksGetResult;
  };
  /**
   * Stored tasks by account, status, class or draft (`feature::TASKS_LIST`).
   */
  'tasks.list': {
    params: TasksListParams;
    result: TasksListResult;
  };
  /**
   * Deletes a template's file, params `TemplateNameParams`.
   * Local clients only.
   */
  'templates.delete': {
    params: TemplateNameParams;
    result: Empty;
  };
  /**
   * One template with its body, params `TemplateNameParams`,
   * result `TemplateResult`.
   */
  'templates.get': {
    params: TemplateNameParams;
    result: TemplateResult;
  };
  /**
   * The templates (`feature::TEMPLATES`), result
   * `TemplatesListResult`.
   */
  'templates.list': {
    params: undefined;
    result: TemplatesListResult;
  };
  /**
   * Saves a template, params `TemplateSaveParams`, result
   * `TemplateResult`. Local clients only.
   */
  'templates.save': {
    params: TemplateSaveParams;
    result: TemplateResult;
  };
  /**
   * `serve` mode, the parent's stdin only, debug builds only: stop one account as a
   * fatal error would (`test-crash` on stdin in `sync` mode).
   */
  'test.crash': {
    params: AccountIdParams;
    result: Empty;
  };
  /**
   * As `TEST_CRASH`, with a panic on the account's `main` thread.
   */
  'test.panic': {
    params: AccountIdParams;
    result: Empty;
  };
  'threads.get': {
    params: ThreadsGetParams;
    result: ThreadsGetResult;
  };
  'threads.list': {
    params: ThreadsListParams;
    result: ThreadsListResult;
  };
  /**
   * Moves threads to Snoozed with the metadata that brings them back
   * (`feature::SNOOZE`), params
   * `ThreadsSnoozeParams`.
   */
  'threads.snooze': {
    params: ThreadsSnoozeParams;
    result: ThreadsSnoozeResult;
  };
  /**
   * Ends a snooze now, params `ThreadsUnsnoozeParams`.
   */
  'threads.unsnooze': {
    params: ThreadsUnsnoozeParams;
    result: ThreadsSnoozeResult;
  };
}
/**
 * `engine.setLogLevel` params.
 */
export interface SetLogLevelParams {
  /**
   * Unix milliseconds; verbose logging is on for accounts started before then. 0 or a
   * past time turns it off.
   */
  verboseUntil: number;
}
/**
 * `engine.setLogLevel` result: the accounts restarted to apply it.
 */
export interface SetLogLevelResult {
  restarted: string[];
}
/**
 * `query.unsubscribe` and `events.unsubscribe`.
 */
export interface SubIdParams {
  subId: string;
}
/**
 * `rules.cancel`: stops a `rules.reprocess` job before its next batch of threads.
 */
export interface RulesCancelParams {
  jobId: string;
}
export interface RulesCancelResult {
  /**
   * False when no such job was running.
   */
  cancelled: boolean;
}
/**
 * `rules.reprocess`: applies the account's enabled rules to the mail in its inbox, newest
 * threads first.
 */
export interface RulesReprocessParams {
  accountId: string;
}
export interface RulesReprocessResult {
  jobId: string;
  /**
   * The inbox threads the job goes through.
   */
  total: number;
}
/**
 * `search` (over `ThreadSearch`). Without `grammar`, `query` is plain text: each
 * whitespace-separated term must match, and nothing in it is read as FTS syntax. With
 * `grammar: true` (feature `search-grammar`) it is the search grammar, its date terms read
 * in `tz` (an IANA name, UTC when absent) and counted back from `now` (unix seconds; needed
 * only by relative dates); text that doesn't parse, or a date outside the documented
 * subset, is `INVALID_PARAMS`. The threads are `accountId`'s, or `accounts`' (feature
 * `search-grammar`), or, with neither, every account's the engine holds.
 */
export interface SearchParams {
  accountId?: string | null;
  accounts?: string[] | null;
  grammar?: boolean;
  limit?: number | null;
  now?: number | null;
  query: string;
  tz?: string | null;
}
/**
 * `search`: matching threads, newest first.
 */
export interface SearchResult {
  threads: ThreadJson[];
}
export interface SecretsDeleteParams {
  name: string;
}
/**
 * `secrets.has` params; the result maps each name to whether a value is stored.
 */
export interface SecretsHasParams {
  names: string[];
}
/**
 * `secrets.set` params. Names: `integration:<provider>`, `ai:<provider>`,
 * `tern-server-token` and the like; an account's secrets go through `account.*`.
 */
export interface SecretsSetParams {
  name: string;
  value: string;
}
/**
 * `server.set` and `server.clear` result: the accounts restarted to pick up the server.
 */
export interface ServerSetResult {
  restarted: string[];
}
/**
 * `server.set` params: the Tern server the engine's accounts sync metadata with. Its token
 * is the secret `tern-server-token` (`secrets.set`). Running accounts restart with it.
 */
export interface ServerSetParams {
  serverId: string;
  url: string;
  userId: string;
}
/**
 * `settings.get` params. Without `keys`, every stored value.
 */
export interface SettingsGetParams {
  keys?: string[] | null;
}
/**
 * `settings.get` result, `settings.set` params and `settings.changed` params. In
 * `settings.set`, null clears a key; in `settings.changed`, null means it was cleared.
 */
export interface SettingsValues {
  /**
   * `settings.get` only: the store's id, new whenever the engine starts a store from
   * nothing (no file, or a corrupt one moved aside). A client that copied its values into
   * the store copies them again when the id changes, rather than taking the missing keys
   * as cleared.
   */
  storeId?: string | null;
  values: {
    [k: string]: unknown;
  };
}
/**
 * `null` (or absent) means every account, or every model class. `since`
 * (`feature::DELTA_SEQ`) resumes a feed: the deltas after it
 * that the engine still holds come after the response, instead of the backlog an
 * unfiltered first subscriber takes.
 */
export interface SubscribeParams {
  accounts?: string[] | null;
  classes?: string[] | null;
  since?: DeltaCursor | null;
}
/**
 * `subscribe`'s result. `feed` names this engine process's delta sequence; `seq` is the
 * last sequence number it gave out before the call. With `since`, `resumed` says whether
 * every delta after it that the filter takes follows the response (seq at most `seq`,
 * then live ones); false means the engine no longer holds them, or `since` is another
 * engine's, and the client reads again what it shows.
 */
export interface SubscribeResult {
  feed: string;
  resumed?: boolean;
  seq: number;
}
export interface SyncWakeParams {
  /**
   * `None` wakes every account.
   */
  accountId?: string | null;
}
export interface TaskCancelParams {
  accountId: string;
  taskId: string;
}
export interface TaskQueueParams {
  accountId: string;
  /**
   * Send `task.done` on this connection when the task finishes
   * (`feature::TASK_DONE`).
   */
  notify?: boolean;
  /**
   * The task JSON the desktop sends in `queue-task`.
   */
  task: TaskJson;
}
/**
 * `tasks.get`: a task of the account by id.
 */
export interface TasksGetParams {
  accountId: string;
  id: string;
}
/**
 * The stored `Task` JSON. Finished tasks are kept until the newest hundred of the account
 * push them out (`cleanupOldTasksAtRuntime`), so a missing task may have finished long ago.
 */
export interface TasksGetResult {
  task: TaskJson;
}
/**
 * `tasks.list`: the stored tasks, the pending ones (`local`, `remote`) first, oldest queued
 * first, then the finished ones, newest first. Every filter is optional: no
 * `accountId` lists every account the engine holds, no `status` every status (`local`,
 * `remote`, `complete`, `cancelled`). `headerMessageId` keeps the tasks naming that draft,
 * in their `headerMessageId` or their `draft`'s.
 */
export interface TasksListParams {
  accountId?: string | null;
  /**
   * The task's `__cls`, e.g. `SendDraftTask`.
   */
  class?: string | null;
  headerMessageId?: string | null;
  limit?: number | null;
  status?: string[] | null;
}
export interface TasksListResult {
  tasks: TaskJson[];
}
/**
 * `templates.get` and `templates.delete`.
 */
export interface TemplateNameParams {
  /**
   * A template's name or its file name.
   */
  name: string;
}
export interface TemplateResult {
  template: TemplateInfo;
}
/**
 * One template as `templates.list` lists it.
 */
export interface TemplateInfo {
  /**
   * HTML; `templates.get` and `templates.save` only.
   */
  body?: string;
  /**
   * The file name, the desktop's template id.
   */
  id: string;
  /**
   * The name without the extension; what `template` params take.
   */
  name: string;
  /**
   * The body as one line of text, cut short.
   */
  preview: string;
  /**
   * Empty when it has none.
   */
  subject: string;
  /**
   * The tagged variables (`<span data-tvar="name">`), in order.
   */
  variables: string[];
}
export interface TemplatesListResult {
  /**
   * By name.
   */
  templates: TemplateInfo[];
}
/**
 * `templates.save`: from `body` (and `subject`) under `name`, or from a draft as the
 * desktop's "Save as template" makes one: the text above its signature and quote, named
 * after its subject (or `name`), with its subject unless it is a reply or a forward. A name
 * already taken gets ` 1`, ` 2`, … unless `replace`.
 */
export interface TemplateSaveParams {
  /**
   * The draft to save, with `messageId` (its id).
   */
  accountId?: string | null;
  /**
   * HTML.
   */
  body?: string | null;
  messageId?: string | null;
  /**
   * Letters, digits, spaces, `-` and `_`.
   */
  name?: string | null;
  /**
   * Write over the template of that name instead of saving under a new one.
   */
  replace?: boolean;
  subject?: string | null;
}
/**
 * `threads.get`.
 */
export interface ThreadsGetParams {
  accountId: string;
  id: string;
}
/**
 * The thread and its messages, oldest first, without bodies.
 */
export interface ThreadsGetResult {
  messages: MessageJson[];
  thread: ThreadJson;
}
/**
 * `threads.list`. At most one of `folderId`, `labelId` and `role` (a folder's role, or
 * else a label's; not empty); none lists every thread of the account. A category other
 * than spam or trash lists only threads in all mail.
 */
export interface ThreadsListParams {
  accountId: string;
  /**
   * The `next` of the previous page.
   */
  before?: string | null;
  folderId?: string | null;
  labelId?: string | null;
  limit?: number | null;
  role?: string | null;
}
/**
 * Newest first by `(lastMessageTimestamp, id)`. `next` is an opaque cursor for `before`,
 * null on the last page.
 */
export interface ThreadsListResult {
  next?: string | null;
  threads: ThreadJson[];
}
/**
 * `threads.snooze`: move the threads to Snoozed and write the `thread-snooze` metadata that
 * brings them back at `until`, as the desktop's snooze popover does.
 */
export interface ThreadsSnoozeParams {
  accountId: string;
  threadIds: string[];
  /**
   * Unix seconds, in the future.
   */
  until: number;
}
/**
 * `threads.snooze` and `threads.unsnooze`: the tasks queued, the move first. Their local
 * phases have run; each arrives as a `Task` delta like any other.
 */
export interface ThreadsSnoozeResult {
  taskIds: string[];
}
/**
 * `threads.unsnooze`: clear the snooze and move back to the inbox what is still in Snoozed.
 */
export interface ThreadsUnsnoozeParams {
  accountId: string;
  threadIds: string[];
}
/**
 * Each notification's params.
 */
export interface RpcNotifications {
  /**
   * Notification: an account's registered JSON changed, `params` =
   * `AccountChangedParams`. Filtered by account, never held
   * for a later subscriber: a client calls `account.list` after `subscribe`.
   */
  'account.changed': AccountChangedParams;
  /**
   * Notification: an account changed state, `params` =
   * `AccountStateParams`.
   */
  'account.state': AccountStateParams;
  /**
   * Notification: the next piece of a streamed answer, `params` =
   * `AiDeltaParams`. Sent only to the connection that started the
   * stream, in order, without `subscribe`; the call's response comes after the last one
   * and carries the whole text.
   */
  'ai.delta': AiDeltaParams;
  /**
   * Notification: triage progress for the connection that called `ai.triage.run`.
   */
  'ai.triage.progress': AiTriageProgressParams;
  /**
   * Notification: the engine's `capabilities` changed (`feature::CAPABILITIES`),
   * `params` = the new capabilities object, whole. Sent to every subscribed connection
   * whatever its filter, never held for a later subscriber: `hello` and `engine.status`
   * carry the current value.
   */
  'capabilities.changed': Capabilities;
  /**
   * Notification: one flushed delta, `params` = `DeltaParams`.
   */
  delta: DeltaParams;
  /**
   * Notification: the engine is about to close the connection, `params` =
   * `DisconnectParams`.
   */
  disconnect: DisconnectParams;
  /**
   * Notification: an `events.subscribe`'s occurrences changed, `params` =
   * `EventsChangedParams`. To the subscribing connection only.
   */
  'events.changed': EventsChangedParams;
  /**
   * Notification: new mail, `params` = `MailNewParams`. Filtered by
   * account only, never held for a later subscriber (`feature::MAIL_NEW`).
   */
  'mail.new': MailNewParams;
  /**
   * Notification: a live query's window changed, `params` =
   * `QueryChangedParams`. To the subscribing connection only,
   * without `subscribe`.
   */
  'query.changed': QueryChangedParams;
  /**
   * Notification: the engine disabled a failing rule. Sent to every subscribed connection
   * whatever its filter, never held: the rule's `disabled` and `disabledReason` are saved in
   * `mailRules`, and `settings.changed` carries them as well.
   */
  'rules.disabled': RulesDisabledParams;
  /**
   * Notification: a `rules.reprocess` job's progress, to the connection that started it
   * only, without `subscribe`; never held.
   */
  'rules.progress': RulesProgressParams;
  /**
   * Notification: settings another connection (or the parent) changed, `params` =
   * `SettingsValues` with the new values, null for a cleared key.
   * Sent to every subscribed connection whatever its filter, never held for a later
   * subscriber: a client calls `settings.get` after `subscribe`.
   */
  'settings.changed': SettingsValues;
  /**
   * Notification: a task queued on this connection with `task.queue {notify: true}`
   * finished (`feature::TASK_DONE`), `params` =
   * `TaskDoneParams`. Sent without `subscribe`, after the
   * `task.queue` answer.
   */
  'task.done': TaskDoneParams;
}
/**
 * Notification `method::RULES_DISABLED`, to every subscribed
 * connection: the engine disabled a rule that failed. The rule's `disabled` and
 * `disabledReason` are in `mailRules` too, so a client that missed this reads them there.
 */
export interface RulesDisabledParams {
  error: string;
  ruleId: string;
}
/**
 * Notification `method::RULES_PROGRESS`, to the connection
 * that called `rules.reprocess`: `done` of `total` inbox threads gone through. The last one
 * has `done == total`, or `stopped`.
 */
export interface RulesProgressParams {
  accountId: string;
  done: number;
  jobId: string;
  /**
   * The job ended after `done` threads: it was cancelled or the account stopped.
   */
  stopped?: boolean;
  total: number;
}
/**
 * `task.done`: a task queued with `task.queue {notify: true}` on this connection finished.
 */
export interface TaskDoneParams {
  accountId: string;
  /**
   * The `SyncError` JSON of a failed task; null when it succeeded.
   */
  error?: SyncErrorJson | null;
  /**
   * `complete` or `cancelled`.
   */
  status: 'complete' | 'cancelled';
  /**
   * The stored task as it finished.
   */
  task: TaskJson;
  taskId: string;
}
