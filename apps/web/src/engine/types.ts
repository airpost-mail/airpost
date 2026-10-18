// Wire types for the engine's model JSON. Field names are the JSON keys the engine writes
// (crates/ms-store/src/models/*.rs) and the desktop parses (apps/desktop/app/src/flux/models),
// so the mock and the real engine share one set of types. Timestamps are unix seconds.

export type ModelClass =
  | 'Thread'
  | 'Message'
  | 'Folder'
  | 'Label'
  | 'Contact'
  | 'ContactGroup'
  | 'ContactBook'
  | 'Account'
  | 'File'
  | 'Event'
  | 'Calendar'
  | 'Task'
  | 'ProcessState'
  /** The engine's Tern server (backend) rejected its token; carries no models. */
  | 'ProcessIdentityRefreshNeeded';

/** `{id, aid, v}` from `base_json`, plus the class tag the desktop's deserializer reads. */
export interface ModelJSON {
  id: string;
  aid: string;
  v: number;
  __cls: string;
}

export interface PluginMetadataJSON {
  pluginId: string;
  v: number;
  value: unknown;
}

export interface WithMetadata {
  metadata?: PluginMetadataJSON[];
}

export const STANDARD_ROLES = [
  'inbox',
  'important',
  'snoozed',
  'sent',
  'drafts',
  'all',
  'spam',
  'archive',
  'trash',
] as const;

export type StandardRole = (typeof STANDARD_ROLES)[number];
/** Empty string for user folders and labels. */
export type CategoryRole = StandardRole | 'starred' | '';

export interface CategoryJSON extends ModelJSON {
  __cls: 'Folder' | 'Label';
  /** IMAP path, modified UTF-7. */
  path: string;
  role: CategoryRole;
  localStatus?: Record<string, unknown>;
}

export interface FolderJSON extends CategoryJSON {
  __cls: 'Folder';
}

export interface LabelJSON extends CategoryJSON {
  __cls: 'Label';
}

/** A folder or label as embedded in `Thread.folders` / `Thread.labels`. */
export type ThreadCategoryRef = CategoryJSON & {
  /** Messages of this thread in the category. */
  _refs: number;
  /** Unread messages of this thread in the category. */
  _u: number;
};

/** A contact as embedded in messages and threads (`Address::to_contact_json`). */
export interface ContactRef {
  email: string;
  name?: string;
}

export interface ThreadJSON extends ModelJSON, WithMetadata {
  __cls: 'Thread';
  subject: string;
  /** Not written by the Rust engine yet; the desktop model has it. */
  snippet?: string;
  /** Count of unread messages; the desktop coerces it to a boolean. */
  unread: number;
  /** Count of starred messages. */
  starred: number;
  /** lastMessageTimestamp (any message); `threads.list` pages by it. */
  lmt: number;
  /** firstMessageTimestamp */
  fmt: number;
  /** lastMessageSentTimestamp */
  lmst: number;
  /** lastMessageReceivedTimestamp */
  lmrt: number;
  /** Gmail thread id (X-GM-THRID), as a string; absent on IMAP accounts. */
  gThrId?: string;
  inAllMail: boolean;
  attachmentCount: number;
  folders: ThreadCategoryRef[];
  labels: ThreadCategoryRef[];
  participants: ContactRef[];
}

export const PlacementFlags = {
  Unread: 1,
  Starred: 2,
  Draft: 4,
} as const;

export interface FileJSON extends ModelJSON {
  __cls: 'File';
  messageId: string;
  partId?: string;
  /** Set for inline parts, matching `cid:` references in the HTML body. */
  contentId: string | null;
  contentType: string;
  filename: string;
  size: number;
}

export interface EventJSON extends ModelJSON {
  __cls: 'Event';
  /** calendarId */
  cid: string;
  ics: string;
  icsuid: string;
  /** recurrenceId */
  rid?: string;
  status?: string;
  /** recurrenceStart, unix seconds */
  rs: number;
  /** recurrenceEnd, unix seconds */
  re: number;
}

/** ms-store `Calendar` (CalDAV collection). */
export interface CalendarJSON extends ModelJSON {
  __cls: 'Calendar';
  name: string;
  /** `#rrggbb`, lower case. */
  color?: string;
  description?: string;
  read_only?: boolean;
  order?: number;
}

export interface MessageJSON extends ModelJSON, WithMetadata {
  __cls: 'Message';
  subject: string;
  snippet?: string;
  date: number;
  threadId: string;
  /** headerMessageId */
  hMsgId: string;
  /** replyToHeaderMessageId */
  rthMsgId?: string | null;
  /** forwardedHeaderMessageId */
  fwdMsgId?: string | null;
  /** Gmail message id, as a string. */
  gMsgId?: string;
  unread: boolean;
  starred: boolean;
  draft: boolean;
  pristine?: boolean;
  /** True when `body` is plain text rather than HTML. */
  plaintext?: boolean;
  /** Joined from the MessageBody table; absent until the engine has downloaded it. */
  body?: string | null;
  /** folderId -> PlacementFlags bits, one key per live copy. */
  folders?: Record<string, number>;
  /** X-GM-LABELS values (e.g. `\\Inbox`, `Work`). */
  labels?: string[];
  from: ContactRef[];
  to: ContactRef[];
  cc: ContactRef[];
  bcc: ContactRef[];
  replyTo: ContactRef[];
  files: FileJSON[];
  events?: EventJSON[];
  extraHeaders?: Record<string, string>;
  /** List-Unsubscribe */
  hListUnsub?: string;
  /** List-Unsubscribe-Post */
  hListUnsubPost?: string;
}

export interface ContactJSON extends ModelJSON {
  __cls: 'Contact';
  name: string;
  email: string;
  /** source: 'mail', 'carddav', 'gpeople' */
  s?: string;
  /** hidden */
  h?: boolean;
  refs: number;
  /** contactGroups ids */
  gis: string[];
  info: Record<string, unknown>;
}

export type SecurityMode = 'SSL / TLS' | 'STARTTLS' | 'none';

export interface AccountSettingsJSON {
  imap_host?: string;
  imap_port?: number;
  imap_username?: string;
  imap_password?: string;
  imap_allow_insecure_ssl?: boolean;
  imap_security?: SecurityMode;
  smtp_host?: string;
  smtp_port?: number;
  smtp_username?: string;
  smtp_password?: string;
  smtp_allow_insecure_ssl?: boolean;
  smtp_security?: SecurityMode;
  refresh_token?: string;
  container_folder?: string;
}

export type SyncState = 'ok' | 'invalid' | 'sync_error';

export interface AccountJSON extends ModelJSON, WithMetadata {
  __cls: 'Account';
  name: string;
  /** 'gmail', 'office365', 'outlook', 'imap', ... */
  provider: string;
  emailAddress: string;
  label: string;
  settings: AccountSettingsJSON;
  autoaddress: { value: string; type: 'cc' | 'bcc' };
  aliases: string[];
  defaultAlias: string | null;
  syncState: SyncState;
  syncError: { code?: number; error?: string; [key: string]: unknown } | null;
  /** unix seconds */
  authedAt?: number;
  color: string;
}

export type TaskStatus = 'local' | 'remote' | 'complete' | 'cancelled';

export interface TaskBaseJSON extends ModelJSON {
  status: TaskStatus;
  source?: string;
  error?: { key?: string; debuginfo?: string; retryable?: boolean; [key: string]: unknown } | null;
  should_cancel?: boolean;
}

interface ChangeMailTaskFields {
  taskDescription?: string;
  threadIds: string[];
  messageIds: string[];
  canBeUndone?: boolean;
  isUndo?: boolean;
}

export interface ChangeUnreadTaskJSON extends TaskBaseJSON, ChangeMailTaskFields {
  __cls: 'ChangeUnreadTask';
  unread: boolean;
}

export interface ChangeStarredTaskJSON extends TaskBaseJSON, ChangeMailTaskFields {
  __cls: 'ChangeStarredTask';
  starred: boolean;
}

export interface ChangeFolderTaskJSON extends TaskBaseJSON, ChangeMailTaskFields {
  __cls: 'ChangeFolderTask';
  folder: FolderJSON;
  sourceFolderIds?: string[];
  undoPlacements?: Record<string, unknown>;
  restorePlacements?: Record<string, unknown>;
}

export interface ChangeLabelsTaskJSON extends TaskBaseJSON, ChangeMailTaskFields {
  __cls: 'ChangeLabelsTask';
  labelsToAdd: LabelJSON[];
  labelsToRemove: LabelJSON[];
}

export interface DestroyDraftTaskJSON extends TaskBaseJSON {
  __cls: 'DestroyDraftTask';
  messageIds: string[];
}

export interface SyncbackDraftTaskJSON extends TaskBaseJSON {
  __cls: 'SyncbackDraftTask';
  headerMessageId: string;
  draft: MessageJSON;
}

export interface SendDraftTaskJSON extends TaskBaseJSON {
  __cls: 'SendDraftTask';
  headerMessageId: string;
  draft: MessageJSON;
  perRecipientBodies?: Record<string, string>;
  silent?: boolean;
}

export interface SyncbackMetadataTaskJSON extends TaskBaseJSON {
  __cls: 'SyncbackMetadataTask';
  pluginId: string;
  modelId: string;
  modelClassName: string;
  modelHeaderMessageId?: string;
  value: unknown;
  undoValue?: unknown;
}

export interface ExpungeAllInFolderTaskJSON extends TaskBaseJSON {
  __cls: 'ExpungeAllInFolderTask';
  folder: FolderJSON;
}

export interface SyncbackCategoryTaskJSON extends TaskBaseJSON {
  __cls: 'SyncbackCategoryTask';
  path: string;
  existingPath?: string;
  created?: CategoryJSON | null;
}

export interface DestroyCategoryTaskJSON extends TaskBaseJSON {
  __cls: 'DestroyCategoryTask';
  path: string;
}

export interface ChangeRoleMappingTaskJSON extends TaskBaseJSON {
  __cls: 'ChangeRoleMappingTask';
  path: string;
  role: string;
}

export interface GetMessageRFC2822TaskJSON extends TaskBaseJSON {
  __cls: 'GetMessageRFC2822Task';
  messageId: string;
  filepath: string;
}

export interface EventRSVPTaskJSON extends TaskBaseJSON {
  __cls: 'EventRSVPTask';
  ics: string;
  icsRSVPStatus: string;
  to: string;
  subject: string;
  messageId: string;
  method?: string;
  comment?: string;
}

export interface SyncbackContactTaskJSON extends TaskBaseJSON {
  __cls: 'SyncbackContactTask';
  contact: ContactJSON;
}

export interface DestroyContactTaskJSON extends TaskBaseJSON {
  __cls: 'DestroyContactTask';
  contacts: ContactJSON[];
}

export type TaskJSON =
  | ChangeUnreadTaskJSON
  | ChangeStarredTaskJSON
  | ChangeFolderTaskJSON
  | ChangeLabelsTaskJSON
  | DestroyDraftTaskJSON
  | SyncbackDraftTaskJSON
  | SendDraftTaskJSON
  | SyncbackMetadataTaskJSON
  | ExpungeAllInFolderTaskJSON
  | SyncbackCategoryTaskJSON
  | DestroyCategoryTaskJSON
  | ChangeRoleMappingTaskJSON
  | GetMessageRFC2822TaskJSON
  | EventRSVPTaskJSON
  | SyncbackContactTaskJSON
  | DestroyContactTaskJSON;

export type TaskClass = TaskJSON['__cls'];

export type ChangeMailTaskJSON =
  ChangeUnreadTaskJSON | ChangeStarredTaskJSON | ChangeFolderTaskJSON | ChangeLabelsTaskJSON;

export type AnyModelJSON =
  | ThreadJSON
  | MessageJSON
  | FolderJSON
  | LabelJSON
  | ContactJSON
  | AccountJSON
  | FileJSON
  | EventJSON
  | CalendarJSON
  | TaskJSON;

export interface ModelJSONByClass {
  Thread: ThreadJSON;
  Message: MessageJSON;
  Folder: FolderJSON;
  Label: LabelJSON;
  Contact: ContactJSON;
  Account: AccountJSON;
  File: FileJSON;
  Event: EventJSON;
  Task: TaskJSON;
  ContactGroup: ModelJSON;
  ContactBook: ModelJSON;
  Calendar: CalendarJSON;
  ProcessState: ProcessStateJSON;
  ProcessIdentityRefreshNeeded: never;
}

/** Not a model: the engine streams its online/offline state alongside the deltas. */
export interface ProcessStateJSON {
  accountId: string;
  connectionError: boolean;
}

/** One delta notification (`DeltaStream` line plus the account it belongs to). */
export interface Delta<C extends ModelClass = ModelClass> {
  accountId: string;
  modelClass: C;
  /**
   * `metadata-expiration` reports models whose plugin metadata `expiration` has passed (for
   * example snooze or send-later). The models are unchanged; it is not a removal.
   */
  type: 'persist' | 'unpersist' | 'metadata-expiration';
  modelJSONs: ModelJSONByClass[C][];
}
