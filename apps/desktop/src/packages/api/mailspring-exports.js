/* eslint global-require: 0 */
/* eslint import/no-dynamic-require: 0 */
const DatabaseObjectRegistry = require('../../data/database-object-registry').default;

// This module exports an empty object, with a ton of defined properties that
// `require` files the first time they're called.
module.exports = exports = window.$m = {};

const resolveExport = (prop, requireValue) => {
  return requireValue.default || requireValue[prop] || requireValue;
};

const lazyLoadWithGetter = (prop, getter) => {
  const key = `${prop}`;

  if (exports[key]) {
    throw new Error(`Fatal error: Duplicate entry in mailspring-exports: ${key}`);
  }
  Object.defineProperty(exports, prop, {
    configurable: true,
    enumerable: true,
    get: () => {
      const value = getter();
      Object.defineProperty(exports, prop, { enumerable: true, value });
      return value;
    },
  });
};

/*
A deprecated export. Its getter stays a getter, so each
package that reads it is warned once; the app's own code isn't.
*/
const warnDeprecated = (prop, { since, replacement, removeIn }, warned) => {
  const caller = require('../../engine/client/api').callerPackageFromStack(new Error().stack);
  if (caller && !warned.has(caller.name)) {
    warned.add(caller.name);
    console.warn(
      `${caller.name} uses ${prop}, deprecated since plugin API ${since} and removed in ${removeIn}: ${replacement}`
    );
  }
};

const deprecate = (prop, info, getter) => {
  let value;
  let loaded = false;
  const warned = new Set();
  Object.defineProperty(exports, prop, {
    configurable: true,
    enumerable: true,
    get: () => {
      warnDeprecated(prop, info, warned);
      if (!loaded) {
        value = getter();
        loaded = true;
      }
      return value;
    },
  });
};

/*
An export some of whose members are deprecated: each of them warns as deprecate() does, once
per package that reads it, and is `stub` from then on (a stand-in, or the member that replaces it).
*/
const withDeprecatedMembers = (prop, value, members) => {
  const out = Object.assign({}, value);
  for (const [name, { stub, ...info }] of Object.entries(members)) {
    const warned = new Set();
    Object.defineProperty(out, name, {
      enumerable: true,
      get: () => {
        warnDeprecated(`${prop}.${name}`, info, warned);
        return stub;
      },
    });
  }
  return out;
};

const lazyLoad = (prop, path) => {
  lazyLoadWithGetter(prop, () => resolveExport(prop, require(`../../${path}`)));
};

const _resolveNow = [];
const load = (klassName, path) => {
  lazyLoad(klassName, path);
  _resolveNow.push(klassName);
};

const lazyLoadAndRegisterModel = (klassName, path) => {
  lazyLoad(klassName, `data/models/${path}`);
  DatabaseObjectRegistry.register(klassName, () => exports[klassName]);
};

const lazyLoadAndRegisterTask = (klassName, path) => {
  lazyLoad(klassName, `data/tasks/${path}`);
  DatabaseObjectRegistry.register(klassName, () => exports[klassName]);
};

lazyLoadWithGetter(`localized`, () => require('../../intl/localize').localized);
lazyLoadWithGetter(
  `localizedReactFragment`,
  () => require('../../intl/localize').localizedReactFragment
);
lazyLoadWithGetter(
  `getAvailableLanguages`,
  () => require('../../intl/localize').getAvailableLanguages
);
lazyLoadWithGetter(`getCurrentLocale`, () => require('../../intl/localize').getCurrentLocale);
lazyLoadWithGetter(`isRTL`, () => require('../../intl/localize').isRTL);

// The plugin API version plugins match `engines.mailspring` against
lazyLoadWithGetter(`apiVersion`, () => require('../api-version').PLUGIN_API_VERSION);

// Actions
lazyLoad(`Actions`, 'data/actions');

// Sync engine
lazyLoad(`MailsyncProcess`, 'engine/process');
// The Database
lazyLoad(`Matcher`, 'data/attributes/matcher');
lazyLoadWithGetter(
  'AndCompositeMatcher',
  () => require('../../data/attributes/matcher').AndCompositeMatcher
);
lazyLoadWithGetter(
  'OrCompositeMatcher',
  () => require('../../data/attributes/matcher').OrCompositeMatcher
);
lazyLoad(`DatabaseStore`, 'data/database/store');
lazyLoad(`QueryResultSet`, 'data/query/result-set');
lazyLoad(`QuerySubscription`, 'data/query/subscription');
lazyLoad(`MutableQueryResultSet`, 'data/query/mutable-result-set');
lazyLoad(`QuerySubscriptionPool`, 'data/query/subscription-pool');
lazyLoad(`ObservableListDataSource`, 'components/list/observable-data-source');
lazyLoad(`MutableQuerySubscription`, 'data/query/mutable-subscription');

// Database Objects
exports.DatabaseObjectRegistry = DatabaseObjectRegistry;
lazyLoad(`Model`, 'data/models/model');
lazyLoad(`Attributes`, 'data/attributes');
lazyLoadAndRegisterModel(`File`, 'file');
lazyLoadAndRegisterModel(`Event`, 'event');
lazyLoadAndRegisterModel(`Label`, 'label');
lazyLoadAndRegisterModel(`Folder`, 'folder');
lazyLoadAndRegisterModel(`Thread`, 'thread');
lazyLoadAndRegisterModel(`Account`, 'account');
lazyLoadAndRegisterModel(`Message`, 'message');
lazyLoadAndRegisterModel(`Contact`, 'contact');
lazyLoadAndRegisterModel(`ContactBook`, 'contact-book');
lazyLoadAndRegisterModel(`ContactGroup`, 'contact-group');
lazyLoadAndRegisterModel(`Category`, 'category');
lazyLoadAndRegisterModel(`Calendar`, 'calendar');
lazyLoadAndRegisterModel(`ProviderSyncbackRequest`, 'provider-syncback-request');

// Search Query Interfaces
lazyLoad(`SearchQueryAST`, 'mail/search/ast');
lazyLoad(`SearchQueryParser`, 'mail/search/parser');
lazyLoad(`IMAPSearchQueryBackend`, 'mail/search/backend-imap');

// Tasks
lazyLoad(`TaskFactory`, 'data/tasks/factory');
lazyLoadAndRegisterTask(`Task`, 'task');
lazyLoadAndRegisterTask(`EventRSVPTask`, 'event-rsvp');
lazyLoadAndRegisterTask(`SendDraftTask`, 'send-draft');
lazyLoadAndRegisterTask(`ChangeMailTask`, 'change-mail');
lazyLoadAndRegisterTask(`DestroyDraftTask`, 'destroy-draft');
lazyLoadAndRegisterTask(`ChangeLabelsTask`, 'change-labels');
lazyLoadAndRegisterTask(`ChangeFolderTask`, 'change-folder');
lazyLoadAndRegisterTask(`ChangeUnreadTask`, 'change-unread');
lazyLoadAndRegisterTask(`DestroyModelTask`, 'destroy-model');
lazyLoadAndRegisterTask(`SyncbackDraftTask`, 'syncback-draft');
lazyLoadAndRegisterTask(`ChangeStarredTask`, 'change-starred');
lazyLoadAndRegisterTask(`SyncbackEventTask`, 'syncback-event');
lazyLoadAndRegisterTask(`DestroyEventTask`, 'destroy-event');
lazyLoadAndRegisterTask(`DestroyCategoryTask`, 'destroy-category');
lazyLoadAndRegisterTask(`SyncbackCategoryTask`, 'syncback-category');
lazyLoadAndRegisterTask(`SyncbackMetadataTask`, 'syncback-metadata');
lazyLoadAndRegisterTask(`GetMessageRFC2822Task`, 'get-message-rfc2822');
lazyLoadAndRegisterTask(`GetManyRFC2822Task`, 'get-many-rfc2822');
lazyLoadAndRegisterTask(`ExpungeAllInFolderTask`, 'expunge-all-in-folder');
lazyLoadAndRegisterTask(`ChangeRoleMappingTask`, 'change-role-mapping');
lazyLoadAndRegisterTask(`SendFeatureUsageEventTask`, 'send-feature-usage-event');
lazyLoadAndRegisterTask(`DestroyContactTask`, 'destroy-contact');
lazyLoadAndRegisterTask(`SyncbackContactTask`, 'syncback-contact');
lazyLoadAndRegisterTask(`DestroyContactGroupTask`, 'destroy-contactgroup');
lazyLoadAndRegisterTask(`SyncbackContactGroupTask`, 'syncback-contactgroup');
lazyLoadAndRegisterTask(`ChangeContactGroupMembershipTask`, 'change-contactgroup-membership');

// Stores
// These need to be required immediately since some Stores are
// listen-only and not explicitly required from anywhere. Stores
// currently set themselves up on require.
load(`TaskQueue`, 'data/stores/task-queue');
load(`BadgeStore`, 'data/stores/badge');
load(`DraftStore`, 'data/drafts/store');
load(`DraftEditingSession`, 'data/drafts/editing-session');
load(`DraftFactory`, 'data/drafts/factory');
load(`ModalStore`, 'data/stores/modal');
load(`OutboxStore`, 'data/stores/outbox');
load(`PopoverStore`, 'data/stores/popover');
load(`AccountStore`, 'data/stores/account');
load(`SignatureStore`, 'data/stores/signature');
load(`MessageStore`, 'data/stores/message');
load(`ContactStore`, 'data/stores/contact');
load(`CategoryStore`, 'data/stores/category');
load(`LabelColorStore`, 'data/stores/label-color');
load(`UndoRedoStore`, 'data/stores/undo-redo');
load(`WorkspaceStore`, 'data/stores/workspace');
load(`MailRulesStore`, 'data/stores/mail-rules');
load(`SendActionsStore`, 'data/stores/send-actions');
load(`ThreadCountsStore`, 'data/stores/thread-counts');
load(`AttachmentStore`, 'data/stores/attachment');
load(`OnlineStatusStore`, 'data/stores/online-status');
load(`PreferencesUIStore`, 'data/stores/preferences-ui');
load(`FocusedContentStore`, 'data/stores/focused-content');
load(`MessageBodyProcessor`, 'mail/body/processor');
load(`FocusedContactsStore`, 'data/stores/focused-contacts');
load(`FolderSyncProgressStore`, 'data/stores/folder-sync-progress');
load(`FocusedPerspectiveStore`, 'data/stores/focused-perspective');
load(`SearchableComponentStore`, 'data/stores/searchable-component');
lazyLoad(`CapabilityStore`, 'data/stores/capability');
lazyLoadWithGetter(`useCapability`, () => require('../../data/stores/capability').useCapability);

lazyLoad(`ServiceRegistry`, `packages/service-registry`);

// Decorators
lazyLoad(`InflatesDraftClientId`, 'components/decorators/inflates-draft-client-id');

// Extensions
lazyLoad(`ExtensionRegistry`, 'packages/extension-registry');
lazyLoad(`MessageViewExtension`, 'packages/message-view-extension');
lazyLoad(`ComposerExtension`, 'packages/composer-extension');

// 3rd party libraries
lazyLoadWithGetter(`Rx`, () => require('rx-lite'));
lazyLoadWithGetter(`React`, () => require('react'));
lazyLoadWithGetter(`ReactDOM`, () => require('react-dom'));
lazyLoadWithGetter(`ReactTestUtils`, () => require('react-dom/test-utils'));
lazyLoadWithGetter(`PropTypes`, () => require('prop-types'));

// React Components
lazyLoad(`ComponentRegistry`, 'packages/component-registry');
lazyLoadWithGetter(
  `SheetDepthContext`,
  () => require('../../components/sheets/context').SheetDepthContext
);

// Utils
lazyLoad(`Utils`, 'data/models/utils');
lazyLoad(`DOMUtils`, 'utils/dom');
lazyLoad(`DateUtils`, 'utils/date');
lazyLoadWithGetter(`imapUtf7`, () => require('../../mail/imap-utf7').imapUtf7);
const vtimezoneGone = (name) => () => {
  throw new Error(
    `CalendarUtils.${name} is gone: the engine writes the VTIMEZONEs of the events it builds (Engine.request("events.create") and "events.update")`
  );
};
lazyLoadWithGetter(`CalendarUtils`, () =>
  withDeprecatedMembers('CalendarUtils', require('../../data/calendar/ics'), {
    resolveIanaZone: {
      since: '1.32.0',
      removeIn: '2.0.0',
      replacement: 'the engine resolves time zones for the events it builds and edits',
      stub: vtimezoneGone('resolveIanaZone'),
    },
    createVTIMEZONEString: {
      since: '1.32.0',
      removeIn: '2.0.0',
      replacement:
        'the engine writes the VTIMEZONEs of the events it builds (Engine.request("events.create") and "events.update")',
      stub: vtimezoneGone('createVTIMEZONEString'),
    },
  })
);
lazyLoad(`CalendarDateUtils`, 'data/calendar/calendar-date');
deprecate(
  `ICSEventHelpers`,
  {
    since: '1.32.0',
    removeIn: '2.0.0',
    replacement:
      'the engine builds and edits events (hello.features "calendar-editing"); use Engine.request("events.expand"), "events.create", "events.update" or "events.rsvp"',
  },
  () =>
    // eslint-disable-next-line no-undef
    new Proxy(
      {},
      {
        // Symbols and `then` stay undefined, so logging or awaiting the stub doesn't throw.
        get: (_target, name) =>
          typeof name !== 'string' || name === 'then'
            ? undefined
            : () => {
                throw new Error(
                  `ICSEventHelpers.${name} is gone: use Engine.request("events.update") and the other events.* methods`
                );
              },
      }
    )
);
lazyLoad(`FsUtils`, 'utils/fs');
lazyLoad(`CanvasUtils`, 'components/drag-canvas');
lazyLoad(`DragDropTypes`, 'components/drag-drop-types');
lazyLoadWithGetter(`RegExpUtils`, () => {
  const RegExpUtils = require('../../utils/regexp').default;
  return withDeprecatedMembers('RegExpUtils', RegExpUtils, {
    mailspringSignatureRegex: {
      since: '1.34.0',
      removeIn: '2.0.0',
      replacement: 'use RegExpUtils.signatureRegex()',
      stub: RegExpUtils.signatureRegex,
    },
    mailspringCommandRegex: {
      since: '1.34.0',
      removeIn: '2.0.0',
      replacement: 'use RegExpUtils.commandRegex()',
      stub: RegExpUtils.commandRegex,
    },
  });
});
lazyLoad(`MenuHelpers`, 'window/menu-helpers');
lazyLoadWithGetter(
  `VirtualDOMUtils`,
  () => require('../../components/searchable/virtual-dom-parser').VirtualDOMUtils
);
lazyLoad(`Spellchecker`, 'window/spellchecker');
lazyLoad(`MessageUtils`, 'data/models/message-utils');

// Services
lazyLoad(`EmlUtils`, 'mail/eml');
lazyLoad(`MboxUtils`, 'mail/mbox');
lazyLoad(`KeyManager`, 'window/key-manager');
lazyLoad(`Integrations`, 'engine/client/integrations');
lazyLoad(`AI`, 'engine/client/ai');
lazyLoad(`Engine`, 'engine/client/api');
lazyLoad(`SoundRegistry`, 'packages/sound-registry');
lazyLoad(`MailRulesTemplates`, 'data/mail-rules-templates');
deprecate(
  `MailRulesProcessor`,
  {
    since: '1.30.0',
    removeIn: '2.0.0',
    replacement:
      'the engine applies the mail rules to arriving mail itself (hello.features "rules"); run them over an inbox with Engine.request("rules.reprocess")',
  },
  () => ({ processMessages: () => Promise.resolve() })
);
lazyLoad(`MailboxPerspective`, 'data/mailbox-perspective');
lazyLoad(`NativeNotifications`, 'window/platform/native-notifications');
lazyLoad(`Autolink`, 'mail/body/autolinker');
lazyLoad(`SanitizeTransformer`, 'mail/body/sanitize-transformer');
lazyLoad(`QuotedHTMLTransformer`, 'mail/body/quoted-html-transformer');
lazyLoad(`InlineStyleTransformer`, 'mail/body/inline-style-transformer');
lazyLoad(`SearchableComponentMaker`, 'components/searchable/component-maker');

// Composer Editor Plugin APIs
// Used by internal packages that need to interact with editor plugins registered in core
// (e.g. composer-grammar-check activates/deactivates the grammar Slate plugin via this API).
lazyLoadWithGetter(
  `GrammarCheckPluginAPI`,
  () => require('../../components/composer-editor/grammar-check-plugins').GrammarCheckPluginAPI
);

// Errors
lazyLoadWithGetter(`APIError`, () => require('../../data/errors').APIError);

// Process Internals
lazyLoad(`DefaultClientHelper`, 'platform/default-client-helper');
lazyLoad(`SystemStartService`, 'window/platform/system-start-service');

// Testing
lazyLoadWithGetter(`TestUtils`, () => require('../../../tests/support/test-utils').default);
deprecate(
  `MailspringTestUtils`,
  { since: '1.34.0', removeIn: '2.0.0', replacement: 'use TestUtils' },
  () => require('../../../tests/support/test-utils').default
);

process.nextTick(() => {
  let c = 0;
  for (const key of _resolveNow) {
    c += exports[key] ? 1 : 0;
  }
  return c;
});
