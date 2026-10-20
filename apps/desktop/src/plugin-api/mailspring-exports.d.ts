export type localized = typeof import('../shared/intl').localized;
export const localized: localized;
export type getCurrentLocale = typeof import('../shared/intl').getCurrentLocale;
export const getCurrentLocale: getCurrentLocale;
export type localizedReactFragment = typeof import('../shared/intl').localizedReactFragment;
export const localizedReactFragment: localizedReactFragment;
export type getAvailableLanguages = typeof import('../shared/intl').getAvailableLanguages;
export const getAvailableLanguages: getAvailableLanguages;
export type isRTL = typeof import('../shared/intl').isRTL;
export const isRTL: isRTL;

/** The plugin API version, matched as semver against a plugin's `engines.mailspring`. */
export const apiVersion: string;

// Actions
export type Actions = typeof import('../flux/actions');
export const Actions: Actions;

// Sync engine
export * from '../shared/mailsync-process';

// The Database
export * from '../flux/attributes/matcher';
export type DatabaseStore = typeof import('../flux/stores/database-store').default;
export const DatabaseStore: DatabaseStore;
export * from '../flux/stores/database-change-record';
export * from '../flux/models/query-result-set';
export * from '../flux/models/query-subscription';
export * from '../flux/models/mutable-query-result-set';
export type QuerySubscriptionPool = typeof import('../flux/models/query-subscription-pool').default;
export const QuerySubscriptionPool: QuerySubscriptionPool;
export * from '../flux/stores/observable-list-data-source';
export * from '../flux/models/mutable-query-subscription';

// Database Objects
export type DatabaseObjectRegistry =
  typeof import('../registries/database-object-registry').default;
export const DatabaseObjectRegistry: DatabaseObjectRegistry;
export * from '../flux/models/model';
export type Attributes = typeof import('../flux/attributes');
export const Attributes: Attributes;
export * from '../flux/models/file';
export * from '../flux/models/event';
export * from '../flux/models/label';
export * from '../flux/models/folder';
export * from '../flux/models/thread';
export * from '../flux/models/account';
export * from '../flux/models/message';
export * from '../flux/models/contact';
export * from '../flux/models/contact-book';
export * from '../flux/models/contact-group';
export * from '../flux/models/category';
export * from '../flux/models/calendar';
export type ProviderSyncbackRequest = import('../flux/models/provider-syncback-request').default;
export const ProviderSyncbackRequest: ProviderSyncbackRequest;

// Search Query Interfaces
export type SearchQueryAST = typeof import('../services/search/search-query-ast');
export const SearchQueryAST: SearchQueryAST;
export * from '../services/search/search-query-parser';
export type IMAPSearchQueryBackend = import('../services/search/search-query-backend-imap').default;
export const IMAPSearchQueryBackend: IMAPSearchQueryBackend;

// Tasks
export * from '../flux/tasks/task';
export * from '../flux/tasks/task-factory';
export * from '../flux/tasks/event-rsvp-task';
export * from '../flux/tasks/send-draft-task';
export * from '../flux/tasks/change-mail-task';
export * from '../flux/tasks/destroy-draft-task';
export * from '../flux/tasks/change-labels-task';
export * from '../flux/tasks/change-folder-task';
export * from '../flux/tasks/change-unread-task';
export * from '../flux/tasks/destroy-model-task';
export * from '../flux/tasks/destroy-event-task';
export * from '../flux/tasks/syncback-draft-task';
export * from '../flux/tasks/change-starred-task';
export * from '../flux/tasks/syncback-event-task';
export * from '../flux/tasks/destroy-category-task';
export * from '../flux/tasks/syncback-category-task';
export * from '../flux/tasks/syncback-metadata-task';
export * from '../flux/tasks/get-message-rfc2822-task';
export * from '../flux/tasks/get-many-rfc2822-task';
export * from '../flux/tasks/expunge-all-in-folder-task';
export * from '../flux/tasks/change-role-mapping-task';
export * from '../flux/tasks/send-feature-usage-event-task';
export * from '../flux/tasks/syncback-contact-task';
export * from '../flux/tasks/destroy-contact-task';
export * from '../flux/tasks/destroy-contactgroup-task';
export * from '../flux/tasks/syncback-contactgroup-task';
export * from '../flux/tasks/change-contactgroup-membership-task';

// Stores
// These need to be required immediately since some Stores are
// listen-only and not explicitly required from anywhere. Stores
// currently set themselves up on require.
export type TaskQueue = typeof import('../flux/stores/task-queue').default;
export const TaskQueue: TaskQueue;
export type BadgeStore = typeof import('../flux/stores/badge-store').default;
export const BadgeStore: BadgeStore;
export type DraftStore = typeof import('../flux/stores/draft-store').default;
export const DraftStore: DraftStore;
export * from '../flux/stores/draft-editing-session';
export type DraftFactory = typeof import('../flux/stores/draft-factory').default;
export const DraftFactory: DraftFactory;
export type ModalStore = typeof import('../flux/stores/modal-store').default;
export const ModalStore: ModalStore;
export type OutboxStore = typeof import('../flux/stores/outbox-store').default;
export const OutboxStore: OutboxStore;
export type PopoverStore = typeof import('../flux/stores/popover-store').default;
export const PopoverStore: PopoverStore;
export * from '../flux/stores/account-store';
export * from '../flux/stores/signature-store';
export * from '../flux/stores/message-store';
export type ContactStore = typeof import('../flux/stores/contact-store').default;
export const ContactStore: ContactStore;
export type CategoryStore = typeof import('../flux/stores/category-store').default;
export const CategoryStore: CategoryStore;
export type LabelColorStore = typeof import('../flux/stores/label-color-store').default;
export const LabelColorStore: LabelColorStore;
export type UndoRedoStore = typeof import('../flux/stores/undo-redo-store').default;
export const UndoRedoStore: UndoRedoStore;
export type WorkspaceStore = typeof import('../flux/stores/workspace-store').default;
export const WorkspaceStore: WorkspaceStore;
export type MailRulesStore = typeof import('../flux/stores/mail-rules-store').default;
export const MailRulesStore: MailRulesStore;
export * from '../flux/stores/send-actions-store';
export type ThreadCountsStore = typeof import('../flux/stores/thread-counts-store').default;
export const ThreadCountsStore: ThreadCountsStore;
export type AttachmentStore = typeof import('../flux/stores/attachment-store').default;
export const AttachmentStore: AttachmentStore;
export type OnlineStatusStore = typeof import('../flux/stores/online-status-store').default;
export const OnlineStatusStore: OnlineStatusStore;
export * from '../flux/stores/preferences-ui-store';
export type FocusedContentStore = typeof import('../flux/stores/focused-content-store').default;
export const FocusedContentStore: FocusedContentStore;
export type MessageBodyProcessor = typeof import('../flux/stores/message-body-processor').default;
export const MessageBodyProcessor: MessageBodyProcessor;
export type FocusedContactsStore = typeof import('../flux/stores/focused-contacts-store').default;
export const FocusedContactsStore: FocusedContactsStore;
export type FolderSyncProgressStore =
  typeof import('../flux/stores/folder-sync-progress-store').default;
export const FolderSyncProgressStore: FolderSyncProgressStore;
export type FocusedPerspectiveStore =
  typeof import('../flux/stores/focused-perspective-store').default;
export const FocusedPerspectiveStore: FocusedPerspectiveStore;
export type SearchableComponentStore =
  typeof import('../flux/stores/searchable-component-store').default;
export const SearchableComponentStore: SearchableComponentStore;
export type CapabilityStore = typeof import('../flux/stores/capability-store').default;
export const CapabilityStore: CapabilityStore;
export const useCapability: typeof import('../flux/stores/capability-store').useCapability;
export type {
  Capability,
  CapabilityFeature,
  CapabilitySource,
} from '../flux/stores/capability-store';

export type ServiceRegistry = typeof import('../registries/service-registry').default;
export const ServiceRegistry: ServiceRegistry;

// Decorators
export type InflatesDraftClientId =
  typeof import('../components/decorators/inflates-draft-client-id').default;
export const InflatesDraftClientId: InflatesDraftClientId;

// Extensions
export type ExtensionRegistry = typeof import('../registries/extension-registry');
export const ExtensionRegistry: ExtensionRegistry;
export * from '../extensions/message-view-extension';
export type {
  MessageBody,
  MessageBodyElement,
  MessageBodyRect,
} from '../message-frame/message-frame-bridge';
export * from '../extensions/composer-extension';

// 3rd party libraries
export type Rx = typeof import('rx-lite');
export const Rx: Rx;
export type React = typeof import('react');
export const React: React;
export type ReactDOM = typeof import('react-dom');
export const ReactDOM: ReactDOM;
export type ReactTestUtils = typeof import('react-dom/test-utils');
export const ReactTestUtils: ReactTestUtils;
export type PropTypes = typeof import('prop-types');
export const PropTypes: PropTypes;

// React Components
export type ComponentRegistry = typeof import('../registries/component-registry').default;
export const ComponentRegistry: ComponentRegistry;
export const SheetDepthContext: typeof import('../components/sheets/sheet-context').SheetDepthContext;

// Utils
export type Utils = typeof import('../flux/models/utils');
export const Utils: Utils;
export type DOMUtils = typeof import('../utils/dom-utils').default;
export const DOMUtils: DOMUtils;
export type DateUtils = typeof import('../utils/date-utils').default;
export const DateUtils: DateUtils;
export type imapUtf7 = typeof import('../utils/imap-utf7').imapUtf7;
export const imapUtf7: imapUtf7;

export type CalendarUtils = typeof import('../utils/calendar-utils') & {
  /**
   * @deprecated since plugin API 1.32.0, removed in 2.0.0: throws. The engine resolves time
   * zones for the events it builds and edits.
   */
  resolveIanaZone: (...args: unknown[]) => never;
  /**
   * @deprecated since plugin API 1.32.0, removed in 2.0.0: throws. The engine writes the
   * VTIMEZONEs of the events it builds (`Engine.request('events.create')`, `'events.update'`).
   */
  createVTIMEZONEString: (...args: unknown[]) => never;
};
export const CalendarUtils: CalendarUtils;
export { ICSParticipantStatus, ICSParticipant } from '../utils/calendar-utils';

export type CalendarDateUtils = typeof import('../utils/calendar-date');
export const CalendarDateUtils: CalendarDateUtils;
export { CalendarDate } from '../utils/calendar-date';

/**
 * @deprecated since plugin API 1.32.0, removed in 2.0.0: the engine builds and edits events
 * (feature `calendar-editing`). Every function throws; use `Engine.request('events.expand')`,
 * `'events.create'`, `'events.update'` or `'events.rsvp'`.
 */
export const ICSEventHelpers: { [name: string]: (...args: unknown[]) => never };
export type { EventConflictJSON as CalendarConflict } from '../shared/engine-protocol';

export type FsUtils = typeof import('../shared/fs-utils');
export const FsUtils: FsUtils;
export type CanvasUtils = typeof import('../utils/canvas-utils');
export const CanvasUtils: CanvasUtils;
export type DragDropTypes = typeof import('../utils/drag-drop-types');
export const DragDropTypes: DragDropTypes;
export type RegExpUtils = typeof import('../utils/regexp-utils').default;
export const RegExpUtils: RegExpUtils;
export type MenuHelpers = typeof import('../window/menu-helpers');
export const MenuHelpers: MenuHelpers;
export type EmlUtils = typeof import('../services/eml-utils');
export const EmlUtils: EmlUtils;
export type MboxUtils = typeof import('../services/mbox-utils');
export const MboxUtils: MboxUtils;
export type VirtualDOMUtils = typeof import('../utils/virtual-dom-utils').default;
export const VirtualDOMUtils: VirtualDOMUtils;
export type Spellchecker = typeof import('../window/spellchecker').default;
export const Spellchecker: Spellchecker;
export type MessageUtils = typeof import('../flux/models/message-utils').default;
export const MessageUtils: MessageUtils;

// Services
export * from '../services/autolinker';
export type KeyManager = typeof import('../window/key-manager').default;
export const KeyManager: KeyManager;
export type Integrations = typeof import('../engine/integrations').default;
export const Integrations: Integrations;
export type Engine = typeof import('../engine/engine-api').default;
export const Engine: Engine;
export type {
  CallerPackage,
  Disposable as EngineDisposable,
  EngineRequestRefused,
  EngineSubscribeFilter,
} from '../engine/engine-api';
export type { EngineNotification } from '../engine/engine-events';
export type {
  EngineCapabilities,
  EngineCapability,
  AiModelInfo,
  AiTestReport,
  AiTokenUsage,
  AiCompletion,
  AiSuggestion,
  AiDeltaParams,
  AiTriageRunResult,
  AiTriageProgressParams,
  AiTriageListResult,
  AiUsageRow,
  RulesReprocessResult,
  RulesProgressParams,
} from '../shared/engine-protocol';
export type AI = typeof import('../engine/ai').default;
export const AI: AI;
export type {
  AiProvider,
  AiProviderKind,
  AiProviderConfig,
  AiFeature,
  AiFeatureSettings,
  AiSettings,
} from '../engine/ai';
export type {
  IntegrationKind,
  IntegrationProvider,
  IntegrationSettings,
  IntegrationsTransport,
  ProviderConfig,
  Translation,
  GrammarCheck,
  GrammarMatch,
  ContactProfile,
  Person,
  Company,
  TestReport,
} from '../engine/integrations';
export type SoundRegistry = typeof import('../registries/sound-registry').default;
export const SoundRegistry: SoundRegistry;
export type MailRulesTemplates = typeof import('../flux/mail-rules-templates');
export const MailRulesTemplates: MailRulesTemplates;
/**
 * @deprecated since plugin API 1.30.0, removed in 2.0.0: the engine applies the mail rules to
 * arriving mail itself (feature `rules`). `processMessages` does nothing; run the rules over an
 * inbox with `Engine.request('rules.reprocess', {accountId})`.
 */
export interface MailRulesProcessor {
  processMessages(messages: unknown[]): Promise<void>;
}
export const MailRulesProcessor: MailRulesProcessor;
export * from '../flux/mailbox-perspective';
export type NativeNotifications = typeof import('../platform/native-notifications').default;
export const NativeNotifications: NativeNotifications;
export type SanitizeTransformer = typeof import('../services/sanitize-transformer').default;
export const SanitizeTransformer: SanitizeTransformer;
export type QuotedHTMLTransformer = typeof import('../services/quoted-html-transformer').default;
export const QuotedHTMLTransformer: QuotedHTMLTransformer;
export type InlineStyleTransformer = typeof import('../services/inline-style-transformer').default;
export const InlineStyleTransformer: InlineStyleTransformer;
export * from '../searchable-components/searchable-component-maker';

// Composer Editor Plugin APIs
export type GrammarCheckPluginAPI =
  typeof import('../components/composer-editor/grammar-check-plugins').GrammarCheckPluginAPI;
export const GrammarCheckPluginAPI: GrammarCheckPluginAPI;
export type GrammarCheckStoreAPI =
  import('../components/composer-editor/grammar-check-plugins').GrammarCheckStoreAPI;

// Errors
export * from '../flux/errors';

// Process Internals
export * from '../shared/default-client-helper';
export type SystemStartService = typeof import('../platform/system-start-service').default;
export const SystemStartService: SystemStartService;

// Testing
export type MailspringTestUtils = typeof import('../../spec/test-utils').default;
export const MailspringTestUtils: MailspringTestUtils;
