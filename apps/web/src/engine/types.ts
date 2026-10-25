// Wire types for the engine's model JSON, generated from the view structs in crates/tern-ipc
// (packages/client, `@tern/client`) and re-exported under the names the web has used. Field
// names are the JSON keys the engine writes; timestamps are unix seconds. What stays here is
// the web's own: the category role list, and the delta and model unions typed by class.
import type {
  AccountJson,
  AccountSettingsJson,
  Autoaddress,
  ChangeFolderTaskJson,
  ChangeLabelsTaskJson,
  ChangeStarredTaskJson,
  ChangeUnreadTaskJson,
  FolderJson,
  LabelJson,
  ModelJsonByClass,
  PluginMetadataJson,
  ThreadJson,
} from '@tern/client';

export type {
  AccountSettingsJson as AccountSettingsJSON,
  CalendarJson as CalendarJSON,
  ChangeFolderTaskJson as ChangeFolderTaskJSON,
  ChangeLabelsTaskJson as ChangeLabelsTaskJSON,
  ChangeRoleMappingTaskJson as ChangeRoleMappingTaskJSON,
  ChangeStarredTaskJson as ChangeStarredTaskJSON,
  ChangeUnreadTaskJson as ChangeUnreadTaskJSON,
  ContactJson as ContactJSON,
  CategoryRole,
  ContactRef,
  DestroyCategoryTaskJson as DestroyCategoryTaskJSON,
  DestroyContactTaskJson as DestroyContactTaskJSON,
  DestroyDraftTaskJson as DestroyDraftTaskJSON,
  EventJson as EventJSON,
  EventRSVPTaskJson as EventRSVPTaskJSON,
  ExpungeAllInFolderTaskJson as ExpungeAllInFolderTaskJSON,
  FileJson as FileJSON,
  FolderJson as FolderJSON,
  GetMessageRFC2822TaskJson as GetMessageRFC2822TaskJSON,
  LabelJson as LabelJSON,
  MessageJson as MessageJSON,
  PluginMetadataJson as PluginMetadataJSON,
  ProcessStateJson as ProcessStateJSON,
  SendDraftTaskJson as SendDraftTaskJSON,
  SyncbackCategoryTaskJson as SyncbackCategoryTaskJSON,
  SyncbackContactTaskJson as SyncbackContactTaskJSON,
  SyncbackDraftTaskJson as SyncbackDraftTaskJSON,
  SyncbackMetadataTaskJson as SyncbackMetadataTaskJSON,
  TaskJson as TaskJSON,
  TaskStatus,
  ThreadCategoryRef,
} from '@tern/client';
export { PlacementFlag as PlacementFlags } from '@tern/client';

/**
 * An account as the web holds it, every field set: `accountFromListEntry` fills what the
 * engine's copy leaves out.
 */
export type AccountJSON = Omit<AccountJson, AccountFilled> & {
  __cls: 'Account';
  v: number;
  name: string;
  provider: string;
  label: string;
  settings: AccountSettingsJson;
  autoaddress: Autoaddress;
  aliases: string[];
  defaultAlias: string | null;
  syncState: SyncState;
  /** The desktop's `{code, error}`, or the engine's `SyncErrorJson` for a stopped account. */
  syncError: { code?: number; error?: string; key?: string; debuginfo?: string } | null;
  color: string;
};

type AccountFilled =
  | '__cls'
  | 'v'
  | 'name'
  | 'provider'
  | 'label'
  | 'settings'
  | 'autoaddress'
  | 'aliases'
  | 'defaultAlias'
  | 'syncState'
  | 'syncError'
  | 'color';

/** `snippet` is the mock engine's; the engine's Thread JSON has none. */
export type ThreadJSON = ThreadJson & { snippet?: string };

/**
 * A `delta.modelClass`. `Account` deltas come from the mock; the engine sends
 * `account.changed`.
 */
export type ModelClass = keyof ModelJSONByClass;

export interface ModelJSONByClass extends ModelJsonByClass {
  Account: AccountJSON;
  Thread: ThreadJSON;
}

/** `{id, aid, v}` from `base_json`, plus the class tag the desktop's deserializer reads. */
export interface ModelJSON {
  id: string;
  aid: string;
  v: number;
  __cls: string;
}

export interface WithMetadata {
  metadata?: PluginMetadataJson[];
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

export type CategoryJSON = FolderJson | LabelJson;

export type SecurityMode = 'SSL / TLS' | 'STARTTLS' | 'none';

export type SyncState = 'ok' | 'invalid' | 'sync_error';

export type TaskBaseJSON = ModelJsonByClass['Task'];

export type TaskClass = ModelJsonByClass['Task']['__cls'];

export type ChangeMailTaskJSON =
  ChangeUnreadTaskJson | ChangeStarredTaskJson | ChangeFolderTaskJson | ChangeLabelsTaskJson;

export type AnyModelJSON = ModelJSONByClass[Exclude<ModelClass, 'ProcessIdentityRefreshNeeded'>];

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
