export type localized = typeof import('../../shared/intl').localized;
export const localized: localized;
export type getCurrentLocale = typeof import('../../shared/intl').getCurrentLocale;
export const getCurrentLocale: getCurrentLocale;
export type localizedReactFragment = typeof import('../../shared/intl').localizedReactFragment;
export const localizedReactFragment: localizedReactFragment;
export type getAvailableLanguages = typeof import('../../shared/intl').getAvailableLanguages;
export const getAvailableLanguages: getAvailableLanguages;
export type isRTL = typeof import('../../shared/intl').isRTL;
export const isRTL: isRTL;

/** The plugin API version, matched as semver against a plugin's `engines.mailspring`. */
export const apiVersion: string;

// Actions
export type Actions = typeof import('../../data/actions');
export const Actions: Actions;

// Sync engine
export * from '../../shared/engine/process';

// The Database
export * from '../../data/attributes/matcher';
export type DatabaseStore = typeof import('../../data/database/store').default;
export const DatabaseStore: DatabaseStore;
export * from '../../data/database/change-record';
export * from '../../data/query/result-set';
export * from '../../data/query/subscription';
export * from '../../data/query/mutable-result-set';
export type QuerySubscriptionPool = typeof import('../../data/query/subscription-pool').default;
export const QuerySubscriptionPool: QuerySubscriptionPool;
export * from '../../components/list/observable-data-source';
export * from '../../data/query/mutable-subscription';

// Database Objects
export type DatabaseObjectRegistry = typeof import('../../data/database-object-registry').default;
export const DatabaseObjectRegistry: DatabaseObjectRegistry;
export * from '../../data/models/model';
export type Attributes = typeof import('../../data/attributes');
export const Attributes: Attributes;
export * from '../../data/models/file';
export * from '../../data/models/event';
export * from '../../data/models/label';
export * from '../../data/models/folder';
export * from '../../data/models/thread';
export * from '../../data/models/account';
export * from '../../data/models/message';
export * from '../../data/models/contact';
export * from '../../data/models/contact-book';
export * from '../../data/models/contact-group';
export * from '../../data/models/category';
export * from '../../data/models/calendar';
export type ProviderSyncbackRequest = import('../../data/models/provider-syncback-request').default;
export const ProviderSyncbackRequest: ProviderSyncbackRequest;

// Search Query Interfaces
export type SearchQueryAST = typeof import('../../mail/search/ast');
export const SearchQueryAST: SearchQueryAST;
export * from '../../mail/search/parser';
export type IMAPSearchQueryBackend = import('../../mail/search/backend-imap').default;
export const IMAPSearchQueryBackend: IMAPSearchQueryBackend;

// Tasks
export * from '../../data/tasks/task';
export * from '../../data/tasks/factory';
export * from '../../data/tasks/event-rsvp';
export * from '../../data/tasks/send-draft';
export * from '../../data/tasks/change-mail';
export * from '../../data/tasks/destroy-draft';
export * from '../../data/tasks/change-labels';
export * from '../../data/tasks/change-folder';
export * from '../../data/tasks/change-unread';
export * from '../../data/tasks/destroy-model';
export * from '../../data/tasks/destroy-event';
export * from '../../data/tasks/syncback-draft';
export * from '../../data/tasks/change-starred';
export * from '../../data/tasks/syncback-event';
export * from '../../data/tasks/destroy-category';
export * from '../../data/tasks/syncback-category';
export * from '../../data/tasks/syncback-metadata';
export * from '../../data/tasks/get-message-rfc2822';
export * from '../../data/tasks/get-many-rfc2822';
export * from '../../data/tasks/expunge-all-in-folder';
export * from '../../data/tasks/change-role-mapping';
export * from '../../data/tasks/send-feature-usage-event';
export * from '../../data/tasks/syncback-contact';
export * from '../../data/tasks/destroy-contact';
export * from '../../data/tasks/destroy-contactgroup';
export * from '../../data/tasks/syncback-contactgroup';
export * from '../../data/tasks/change-contactgroup-membership';

// Stores
// These need to be required immediately since some Stores are
// listen-only and not explicitly required from anywhere. Stores
// currently set themselves up on require.
export type TaskQueue = typeof import('../../data/stores/task-queue').default;
export const TaskQueue: TaskQueue;
export type BadgeStore = typeof import('../../data/stores/badge').default;
export const BadgeStore: BadgeStore;
export type DraftStore = typeof import('../../data/drafts/store').default;
export const DraftStore: DraftStore;
export * from '../../data/drafts/editing-session';
export type DraftFactory = typeof import('../../data/drafts/factory').default;
export const DraftFactory: DraftFactory;
export type ModalStore = typeof import('../../data/stores/modal').default;
export const ModalStore: ModalStore;
export type OutboxStore = typeof import('../../data/stores/outbox').default;
export const OutboxStore: OutboxStore;
export type PopoverStore = typeof import('../../data/stores/popover').default;
export const PopoverStore: PopoverStore;
export * from '../../data/stores/account';
export * from '../../data/stores/signature';
export * from '../../data/stores/message';
export type ContactStore = typeof import('../../data/stores/contact').default;
export const ContactStore: ContactStore;
export type CategoryStore = typeof import('../../data/stores/category').default;
export const CategoryStore: CategoryStore;
export type LabelColorStore = typeof import('../../data/stores/label-color').default;
export const LabelColorStore: LabelColorStore;
export type UndoRedoStore = typeof import('../../data/stores/undo-redo').default;
export const UndoRedoStore: UndoRedoStore;
export type WorkspaceStore = typeof import('../../data/stores/workspace').default;
export const WorkspaceStore: WorkspaceStore;
export type MailRulesStore = typeof import('../../data/stores/mail-rules').default;
export const MailRulesStore: MailRulesStore;
export * from '../../data/stores/send-actions';
export type ThreadCountsStore = typeof import('../../data/stores/thread-counts').default;
export const ThreadCountsStore: ThreadCountsStore;
export type AttachmentStore = typeof import('../../data/stores/attachment').default;
export const AttachmentStore: AttachmentStore;
export type OnlineStatusStore = typeof import('../../data/stores/online-status').default;
export const OnlineStatusStore: OnlineStatusStore;
export * from '../../data/stores/preferences-ui';
export type FocusedContentStore = typeof import('../../data/stores/focused-content').default;
export const FocusedContentStore: FocusedContentStore;
export type MessageBodyProcessor = typeof import('../../mail/body/processor').default;
export const MessageBodyProcessor: MessageBodyProcessor;
export type FocusedContactsStore = typeof import('../../data/stores/focused-contacts').default;
export const FocusedContactsStore: FocusedContactsStore;
export type FolderSyncProgressStore =
  typeof import('../../data/stores/folder-sync-progress').default;
export const FolderSyncProgressStore: FolderSyncProgressStore;
export type FocusedPerspectiveStore =
  typeof import('../../data/stores/focused-perspective').default;
export const FocusedPerspectiveStore: FocusedPerspectiveStore;
export type SearchableComponentStore =
  typeof import('../../data/stores/searchable-component').default;
export const SearchableComponentStore: SearchableComponentStore;
export type CapabilityStore = typeof import('../../data/stores/capability').default;
export const CapabilityStore: CapabilityStore;
export const useCapability: typeof import('../../data/stores/capability').useCapability;
export type { Capability, CapabilityFeature, CapabilitySource } from '../../data/stores/capability';

export type ServiceRegistry = typeof import('../service-registry').default;
export const ServiceRegistry: ServiceRegistry;

// Decorators
export type InflatesDraftClientId =
  typeof import('../../components/decorators/inflates-draft-client-id').default;
export const InflatesDraftClientId: InflatesDraftClientId;

// Extensions
export type ExtensionRegistry = typeof import('../extension-registry');
export const ExtensionRegistry: ExtensionRegistry;
export * from '../message-view-extension';
export type { MessageBody, MessageBodyElement, MessageBodyRect } from '../../mail/frame/bridge';
export * from '../composer-extension';

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
export type ComponentRegistry = typeof import('../component-registry').default;
export const ComponentRegistry: ComponentRegistry;
export const SheetDepthContext: typeof import('../../components/sheets/context').SheetDepthContext;

// Utils
export type Utils = typeof import('../../data/models/utils');
export const Utils: Utils;
export type DOMUtils = typeof import('../../utils/dom').default;
export const DOMUtils: DOMUtils;
export type DateUtils = typeof import('../../utils/date').default;
export const DateUtils: DateUtils;
export type imapUtf7 = typeof import('../../utils/imap-utf7').imapUtf7;
export const imapUtf7: imapUtf7;

export type CalendarUtils = typeof import('../../utils/calendar') & {
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
export { ICSParticipantStatus, ICSParticipant } from '../../utils/calendar';

export type CalendarDateUtils = typeof import('../../utils/calendar-date');
export const CalendarDateUtils: CalendarDateUtils;
export { CalendarDate } from '../../utils/calendar-date';

/**
 * @deprecated since plugin API 1.32.0, removed in 2.0.0: the engine builds and edits events
 * (feature `calendar-editing`). Every function throws; use `Engine.request('events.expand')`,
 * `'events.create'`, `'events.update'` or `'events.rsvp'`.
 */
export const ICSEventHelpers: { [name: string]: (...args: unknown[]) => never };
export type { EventConflictJSON as CalendarConflict } from '../../shared/engine/protocol';

export type FsUtils = typeof import('../../shared/fs-utils');
export const FsUtils: FsUtils;
export type CanvasUtils = typeof import('../../utils/canvas');
export const CanvasUtils: CanvasUtils;
export type DragDropTypes = typeof import('../../utils/drag-drop-types');
export const DragDropTypes: DragDropTypes;
export type RegExpUtils = typeof import('../../utils/regexp').default;
export const RegExpUtils: RegExpUtils;
export type MenuHelpers = typeof import('../../window/menu-helpers');
export const MenuHelpers: MenuHelpers;
export type EmlUtils = typeof import('../../mail/eml');
export const EmlUtils: EmlUtils;
export type MboxUtils = typeof import('../../mail/mbox');
export const MboxUtils: MboxUtils;
export type VirtualDOMUtils = typeof import('../../utils/virtual-dom').default;
export const VirtualDOMUtils: VirtualDOMUtils;
export type Spellchecker = typeof import('../../window/spellchecker').default;
export const Spellchecker: Spellchecker;
export type MessageUtils = typeof import('../../data/models/message-utils').default;
export const MessageUtils: MessageUtils;

// Services
export * from '../../mail/body/autolinker';
export type KeyManager = typeof import('../../window/key-manager').default;
export const KeyManager: KeyManager;
export type Integrations = typeof import('../../engine/integrations').default;
export const Integrations: Integrations;
export type Engine = typeof import('../../engine/api').default;
export const Engine: Engine;
export type {
  CallerPackage,
  Disposable as EngineDisposable,
  EngineRequestRefused,
  EngineSubscribeFilter,
} from '../../engine/api';
export type { EngineNotification } from '../../engine/events';
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
} from '../../shared/engine/protocol';
export type AI = typeof import('../../engine/ai').default;
export const AI: AI;
export type {
  AiProvider,
  AiProviderKind,
  AiProviderConfig,
  AiFeature,
  AiFeatureSettings,
  AiSettings,
} from '../../engine/ai';
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
} from '../../engine/integrations';
export type SoundRegistry = typeof import('../sound-registry').default;
export const SoundRegistry: SoundRegistry;
export type MailRulesTemplates = typeof import('../../data/mail-rules-templates');
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
export * from '../../data/mailbox-perspective';
export type NativeNotifications =
  typeof import('../../window/platform/native-notifications').default;
export const NativeNotifications: NativeNotifications;
export type SanitizeTransformer = typeof import('../../mail/body/sanitize-transformer').default;
export const SanitizeTransformer: SanitizeTransformer;
export type QuotedHTMLTransformer =
  typeof import('../../mail/body/quoted-html-transformer').default;
export const QuotedHTMLTransformer: QuotedHTMLTransformer;
export type InlineStyleTransformer =
  typeof import('../../mail/body/inline-style-transformer').default;
export const InlineStyleTransformer: InlineStyleTransformer;
export * from '../../components/searchable/component-maker';

// Composer Editor Plugin APIs
export type GrammarCheckPluginAPI =
  typeof import('../../components/composer-editor/grammar-check-plugins').GrammarCheckPluginAPI;
export const GrammarCheckPluginAPI: GrammarCheckPluginAPI;
export type GrammarCheckStoreAPI =
  import('../../components/composer-editor/grammar-check-plugins').GrammarCheckStoreAPI;

// Errors
export * from '../../data/errors';

// Process Internals
export * from '../../shared/default-client-helper';
export type SystemStartService =
  typeof import('../../window/platform/system-start-service').default;
export const SystemStartService: SystemStartService;

// Testing
export type MailspringTestUtils = typeof import('../../../tests/support/test-utils').default;
export const MailspringTestUtils: MailspringTestUtils;
