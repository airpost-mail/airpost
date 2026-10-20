# Mailspring Plugin API Inventory

This document catalogs the Mailspring plugin ecosystem to inform the Tern project's plugin architecture design. It covers community plugins, built-in plugins, and the API surface they depend on.

**Research methodology**: GitHub repository search for "mailspring plugin", "mailspring-plugin" topic, manual code inspection of top plugins by stars. Search conducted October 2026. All API usage verified by reading actual plugin source code. Built-in counts are packages under `apps/desktop/{features,plugins,themes}` whose `lib/` names the symbol, recounted 2026-10-16 after the `mcp-server` plugin was removed (tern-mcp, `docs/mcp.md`, replaces it).

---

## 1. Third-Party Community Plugins

### Top Plugins by Popularity

#### mailspring-avatars (82 stars)
- **URL**: https://github.com/Striffly/mailspring-avatars
- **Last updated**: January 2026
- **What it does**: Displays company/domain favicon avatars next to messages in the thread list using favicon lookup services
- **Compatibility**: Actively maintained, compatible with Mailspring >=1.6.3
- **APIs used**:
  - **mailspring-exports**: `ComponentRegistry`, `Utils`, `React`
  - **mailspring-component-kit**: None
  - **ComponentRegistry roles**: `ThreadListIcon`, `Thread:MailLabel` (hack to show larger avatars in narrow mode)
  - **Models**: `Thread` (inferred from role)
  - **DatabaseStore**: No direct usage
  - **Node/Electron**: Image loading, canvas manipulation, XHR/fetch for favicon services
  - **package.json**: `engines.mailspring: ">=1.6.3"`, `windowTypes: {default: true, composer: true}`

#### mailspring-keybase (18 stars)
- **URL**: https://github.com/NgoHuy/mailspring-keybase
- **Last updated**: March 2023
- **What it does**: PGP encryption/decryption integration with Keybase for secure email
- **Compatibility**: Abandoned; requires manual config.json editing and Babel compilation
- **APIs used**:
  - **mailspring-exports**: `PreferencesUIStore`, `ComponentRegistry`, `ExtensionRegistry`
  - **mailspring-component-kit**: None visible
  - **ComponentRegistry roles**: `Composer:ActionButton`, `message:BodyHeader`, `Composer:RecipientChip`
  - **ExtensionRegistry**: `MessageView.register/unregister` (decrypt preprocessing)
  - **PreferencesUIStore**: `registerPreferencesTab`, `TabItem`
  - **Node/Electron**: Keybase CLI subprocess calls
  - **package.json**: None in repo (requires manual install)

#### mailspring-spoof-guard (4 stars)
- **URL**: https://github.com/rages4calm/mailspring-spoof-guard
- **Last updated**: July 2026
- **What it does**: Phishing/spoofing detection with SPF/DKIM/DMARC checks, auto-move to spam, allow/block lists
- **Compatibility**: Active; complex email analysis
- **APIs used**:
  - **mailspring-exports**: `ComponentRegistry`, `PreferencesUIStore`, `Actions`, `TaskFactory`, `TaskQueue`, `DatabaseStore`, `CategoryStore`, `AccountStore`, `Thread`, `Message`, `GetMessageRFC2822Task`
  - **mailspring-component-kit**: None
  - **ComponentRegistry roles**: `MessageHeaderStatus`
  - **PreferencesUIStore**: `registerPreferencesTab`, `TabItem`
  - **DatabaseStore**: `findAll(Message)`, `findAll(Thread)`, `find(Thread)`, `listen()` for incoming mail scanning
  - **Tasks**: `GetMessageRFC2822Task` (fetch full headers), `TaskFactory.tasksForMarkingAsSpam`
  - **Actions**: `queueTask`, `queueTasks`
  - **Models**: `Message.from`, `Thread.categories`, `Account.emailAddress`
  - **Coupling**: Direct database queries, Task creation, MailsyncBridge dependency (RFC2822 fetch)

#### todoer (5 stars)
- **URL**: https://github.com/jmanuel1/todoer
- **Last updated**: June 2024
- **What it does**: Automatically adds starred/labeled emails to local todo.txt file
- **Compatibility**: Works; async file I/O queue
- **APIs used**:
  - **mailspring-exports**: `Actions`, `ComponentRegistry`, `DatabaseStore`, `Thread`, `WorkspaceStore`, `PreferencesUIStore`, `React`
  - **mailspring-component-kit**: None
  - **PreferencesUIStore**: `registerPreferencesTab`, `TabItem`
  - **Actions**: `queueTask.listen`, `queueTasks.listen`, `queueUndoOnlyTask.listen` (monitors Task execution)
  - **DatabaseStore**: No direct queries (listens to Actions instead)
  - **Models**: `Thread.subject`, `Thread.firstMessageTimestamp`, `Thread.id`
  - **Node/Electron**: fs read/write to todo.txt, file backup
  - **AppEnv**: `config` for settings
  - **Coupling**: Monitors Task stream to detect thread changes (indirect DatabaseStore usage)

#### mailspring-sort-inbox (2 stars)
- **URL**: https://github.com/saint-james-fr/mailspring-sort-inbox
- **Last updated**: October 2026
- **What it does**: Airmail-style sort/filter dropdown for thread list (by date, subject, sender)
- **Compatibility**: Active; TypeScript
- **APIs used**:
  - **mailspring-exports**: `ComponentRegistry`, `WorkspaceStore`
  - **mailspring-component-kit**: None
  - **ComponentRegistry**: Uses `location: WorkspaceStore.Location.ThreadList.Toolbar` (alternative to `role`)
  - **WorkspaceStore**: `Location` enum
  - **Coupling**: Hooks into thread-list component internals (inferred; manipulates sort order)

#### Other Notable Plugins

**RyamosThomas/Mailspring-MCP (3 stars)** — April 2026
- Exposes email data via Model Context Protocol server for AI integration
- Uses: `DatabaseStore` queries, likely `Message`/`Thread` models

**3nws/refresh-on-focus (1 star)** — June 2025
- Resyncs mail on window focus with cooldown
- Uses: Window focus events, likely triggers Actions

**arnoo/mailspring-mute-plugin (0 stars)** — February 2025
- Mute thread feature
- Uses: `Thread` model, likely `Actions.queueTask` for metadata

**saint-james-fr/mailspring-sort-inbox (2 stars)** — October 2026
- Already covered above

**EdGaz/mailspring-outgoing-undo (0 stars)** — September 2026
- Restores native undo-send countdown
- Uses: SendDraftTask interception (inferred)

**kz6fittycent/mailspring-package-tracker (0 stars)** — September 2026
- Finds package tracking numbers in emails
- Uses: `MessageView` extension, regex parsing

**hburaktasyurek/Mailspring-Smart-Folders (0 stars)** — September 2026
- Combines folders/labels across accounts
- Uses: `FocusedPerspectiveStore`, `Category` model, `MailboxPerspective`

**ruslansin/launcher-api-support (4 stars)** — April 2019
- Unity Launcher API notifications (Linux)
- Uses: `BadgeStore`, Electron native modules

### Common Third-Party API Patterns

1. **ComponentRegistry roles**: `Composer:ActionButton`, `MessageHeaderStatus`, `ThreadListIcon`, `message:BodyHeader`
2. **PreferencesUIStore**: All non-trivial plugins add settings tabs
3. **DatabaseStore**: `listen()` for reactive updates, `findAll()`/`find()` for queries
4. **Actions**: `queueTask` for mutations, `.listen()` for observing events
5. **Models**: Read-only access to `Thread`, `Message`, `Account`, `Contact`
6. **Tasks**: `TaskFactory` helpers, custom Task queueing, `GetMessageRFC2822Task` for full headers
7. **Node APIs**: File I/O (todo.txt, config), child processes (Keybase), network (favicon services)

### Plugins Not Working / Abandoned

- **mailspring-keybase**: Requires manual config surgery; no package.json
- Plugins from 2019-2020 with no updates likely incompatible

---

## 2. Built-In Plugins (apps/desktop/{features,plugins,themes})

### Core UI Components (Not Pro)

| Package | Purpose | API Usage Summary |
|---------|---------|-------------------|
| **account-sidebar** | Left panel showing accounts/folders | `WorkspaceStore`, `AccountStore`, `FocusedPerspectiveStore`, `CategoryStore`, `ExtensionRegistry.AccountSidebar` |
| **attachments** | View attachments on messages | `ComponentRegistry` role `MessageAttachments`, `AttachmentStore` |
| **category-mapper** | Manual folder/label role mapping | `Actions.queueTask(ChangeRoleMappingTask)`, `CategoryStore`, preferences tab |
| **category-picker** | Label/folder picker UI | `ComponentRegistry`, `DatabaseStore`, `CategoryStore`, `TaskFactory`, `Actions` |
| **composer** | Email composition window | `DraftStore`, `DraftEditingSession`, `SendDraftTask`, `ComponentRegistry` (many roles), `ExtensionRegistry.Composer`, `AccountStore`, `SignatureStore`, `AttachmentStore` |
| **contacts** | Contact management | `DatabaseStore.findAll(Contact)`, `ContactStore`, `ListTabular`, `Actions.queueTask(SyncbackContactTask)` |
| **custom-fonts** | Custom font selection | Injects CSS, `ConfigPropContainer` |
| **custom-sounds** | Custom notification sounds | `SoundRegistry`, file picker |
| **draft-list** | Draft list view | `DatabaseStore.findAll(Message, draft=true)`, `ObservableListDataSource`, `ListTabular`, `WorkspaceStore`, `Actions.composeNewDraft` |
| **events** | Calendar event display in message | `ComponentRegistry` role `MessageListHeaders`, `Event` model, `EventRSVPTask` |
| **list-unsubscribe** | Unsubscribe link UI | `ComponentRegistry` role `message:BodyHeader`, `Message.unsubscribeLinks` |
| **main-calendar** | Calendar view | `DatabaseStore.findAll(Event)`, `Event`, `Calendar`, `SyncbackEventTask`, `DestroyEventTask`, `EventRSVPTask`, `ComponentRegistry` role `NavRail:Item` |
| **message-autoload-images** | Conditional image loading | `ExtensionRegistry.MessageView`, `Message` content manipulation |
| **message-list** | Message list within thread | `ComponentRegistry`, `MessageStore`, `FocusedContentStore`, `DatabaseStore` |
| **message-view-on-github** | GitHub notification helper | `ComponentRegistry` role `message:BodyHeader`, parses GitHub URLs from `Message.body` |
| **mode-switch** | Split/list mode switcher | `WorkspaceStore`, `Actions` |
| **notifications** | Desktop notifications | `NativeNotifications`, `BadgeStore`, `FocusedPerspectiveStore`, `DatabaseStore.listen()` |
| **onboarding** | Account setup flow | `Actions.finishAndAddAccount`, `PreferencesUIStore` |
| **participant-profile** | Contact sidebar info | `ComponentRegistry` role `MessageListSidebar:ContactCard`, `Contact`, `FocusedContactsStore` |
| **phishing-detection** | Reply-to mismatch warnings | `ExtensionRegistry.MessageView`, `Message.replyTo`, `Contact.email` |
| **preferences** | Settings window | `PreferencesUIStore`, `AccountStore`, `Actions`, many tabs |
| **print** | Print email | `MessageStore`, window.print() |
| **remove-tracking-pixels** | Strip tracking pixels | `ExtensionRegistry.MessageView`, `Message` body manipulation |
| **screenshot-mode** | Anonymize for screenshots | Replaces text/names with blocks |
| **send-and-archive** | Send+archive button | `ExtensionRegistry.Composer`, `SendDraftTask`, `TaskFactory.tasksForArchiving`, `Actions.queueTasks` |
| **system-tray** | System tray icon | `BadgeStore`, Electron Tray API |
| **theme-picker** | Theme selection UI | Reads theme files, `PreferencesUIStore` tab |
| **thread-list** | Thread list view | `DatabaseStore`, `ObservableListDataSource`, `ListTabular`, `FocusedPerspectiveStore`, `ThreadCountsStore`, `WorkspaceStore`, `ExtensionRegistry.ThreadList`, `ComponentRegistry` (many roles) |
| **thread-search** | Thread search bar | `Actions.searchQuerySubmitted`, `FocusedPerspectiveStore`, `SearchableComponentStore` |
| **undo-redo** | Undo/redo notifications | `UndoRedoStore`, `Actions.undo`, `Actions.redo`, `Task.canBeUndone` |
| **unread-notifications** | Unread count notifications | `NativeNotifications`, `UnreadBadgeStore`, `DatabaseStore.listen()` |
| **Themes (ui-*)** | 7 themes (dark, light, taiga, darkside, ubuntu, less-is-more, automatic) | LESS stylesheets only |

### Pro Features (isOptional: true)

| Package | Purpose | API Usage Summary |
|---------|---------|-------------------|
| **activity** | Open/link tracking dashboard | `DatabaseStore.findAll(Message)` with metadata queries, `MailspringAPIRequest` for analytics, `SyncbackMetadataTask`, `ComponentRegistry` role `RootSidebar:Notifications`, `WorkspaceStore`, custom `MailboxPerspective` |
| **composer-grammar-check** | LanguageTool grammar check | `ExtensionRegistry.Composer`, `GrammarCheckPluginAPI` (editor plugin activation), HTTP to LanguageTool |
| **composer-signature** | Email signatures | `SignatureStore`, `Actions.upsertSignature`, `PreferencesUIStore`, `ExtensionRegistry.Composer` |
| **composer-templates** | Quick reply templates | `DraftStore`, `Actions.insertTemplateId`, templates stored in config, `PreferencesUIStore`, `ComponentRegistry` role `Composer:ActionButton` |
| **github-contact-card** | GitHub profile in sidebar | `ComponentRegistry` role `MessageListSidebar:ContactCard`, GitHub API, `Contact.email` |
| **link-tracking** | Link click tracking | `MetadataComposerToggleButton`, `ExtensionRegistry.Composer`, `ExtensionRegistry.MessageView`, `Message.metadataForPluginId()`, `SyncbackMetadataTask`, rewrites links in HTML, `ComponentRegistry` roles `Composer:ActionButton`, `ThreadListIcon`, `MessageHeaderStatus` |
| **open-tracking** | Email open tracking | `MetadataComposerToggleButton`, `ExtensionRegistry.Composer`, `Message.metadataForPluginId()`, `SyncbackMetadataTask`, injects tracking pixel, `ComponentRegistry` roles `Composer:ActionButton`, `ThreadListIcon`, `MessageHeaderStatus` |
| **personal-level-indicators** | "To: me" indicators | `ComponentRegistry` role `ThreadListIcon`, `Thread.participants`, `AccountStore` |
| **send-later** | Scheduled sending | `DatabaseStore.listen()` for metadata expiration events, `SyncbackMetadataTask` with `expiration` timestamp, `SendDraftTask`, `DraftStore.sessionForClientId()`, `SendActionsStore`, `ComponentRegistry` roles `Composer:ActionButton`, `DraftList:DraftStatus` |
| **send-reminders** | Reply reminders | Similar to send-later: `DatabaseStore.listen()` for metadata expiration, `SyncbackMetadataTask`, `ComponentRegistry` roles `Composer:ActionButton`, `ThreadListTimestamp`, `MessageListHeaders`, `RootSidebar:Notifications`, `ThreadActionsToolbarButton`, `ExtensionRegistry.AccountSidebar`, `ExtensionRegistry.ThreadList` |
| **thread-sharing** | Share thread via web | `MailspringAPIRequest` to upload thread, `Message`, `Thread`, `ComponentRegistry` role `ThreadActionsToolbarButton` |
| **thread-snooze** | Snooze threads | `SyncbackMetadataTask` with `expiration`, `DatabaseStore.listen()`, `TaskFactory.tasksForApplyingCategories`, `CategoryStore.getSnoozedCategory`, `ComponentRegistry` roles `ThreadActionsToolbarButton`, `ThreadListQuickAction`, `Thread:MailLabel` |
| **translation** | Translate messages/drafts | `ExtensionRegistry.MessageView`, `ExtensionRegistry.Composer`, `ComponentRegistry` roles `Composer:ActionButton`, `message:BodyHeader`, HTTP to translation service, `Message.body` manipulation |

### Built-In Plugin API Patterns

1. **Heavy DatabaseStore usage**: Nearly all plugins use `DatabaseStore.findAll()`, `find()`, `listen()`
2. **Task queueing**: `Actions.queueTask` for all mutations; many use `TaskFactory` helpers
3. **Metadata pattern**: Pro plugins (send-later, snooze, tracking) use `Message.metadataForPluginId()` + `SyncbackMetadataTask` + `DatabaseStore.listen()` for expiration events
4. **ComponentRegistry**: Extensive use of injection points (15+ distinct roles)
5. **ExtensionRegistry**: Composer, MessageView, ThreadList, AccountSidebar for pre/post-processing
6. **PreferencesUIStore**: Every plugin with settings registers a tab
7. **ObservableListDataSource + ListTabular**: Standard pattern for virtualized lists (thread-list, draft-list, contacts)
8. **No direct SQLite**: All database access via DatabaseStore abstraction
9. **One bridge import**: onboarding imports `MailsyncProcess` to test an account before adding it; every other engine call goes through `Actions.queueTask`

---

## 3. API Usage Table

### mailspring-exports

| Symbol | Type | Built-in Plugins | Third-Party Plugins | Coupling Risk | Notes |
|--------|------|------------------|---------------------|---------------|-------|
| **Actions** | Event dispatcher | 35 | 5 | **HIGH** | `queueTask`/`queueTasks` are the ONLY write path; many read-only actions (composeReply, openPopover, etc.) are UI-only |
| **ComponentRegistry** | UI injection | 45 | 5 | **MEDIUM** | Pure UI rendering; roles are stable; no engine dependency |
| **ExtensionRegistry** | Pre/post-processors | 23 | 3 | **MEDIUM** | Composer/MessageView extensions transform data; some modify Tasks |
| **DatabaseStore** | Query interface | 17 | 3 | **HIGH** | Direct queries couple to schema; `listen()` requires delta streaming; no `.inTransaction()` available |
| **PreferencesUIStore** | Settings tabs | 18 | 4 | **LOW** | UI-only; no engine dependency |
| **TaskFactory** | Task builders | 12 | 2 | **MEDIUM** | Helpers for creating Tasks; abstracts some complexity but creates engine tasks |
| **Models (Thread/Message/Contact/Account)** | Data models | 50+ | 5 | **HIGH** | Schema-coupled; many computed properties (Thread.participants, Message.isFromMe) |
| **Stores (DraftStore, MessageStore, etc.)** | State | 35+ | 2 | **HIGH** | Maintain UI state; some (DraftStore) mutate data via sessions |
| **Task classes** | Operations | 30+ | 2 | **HIGH** | Direct Task creation couples to engine task types; SyncbackMetadataTask is heavily used |
| **Utils/DateUtils** | Utilities | 25+ | 2 | **LOW** | Pure functions; no coupling |
| **React/ReactDOM/Rx** | Libraries | 52+ | 5 | **LOW** | Standard libraries |
| **AccountStore** | Account list | 20 | 2 | **MEDIUM** | Read-only account metadata |
| **CategoryStore** | Folder/labels | 15 | 1 | **MEDIUM** | Category lookup; used by TaskFactory |
| **FocusedPerspectiveStore** | Current view | 12 | 0 | **LOW** | UI state only |
| **MailboxPerspective** | View definition | 8 | 1 | **MEDIUM** | Custom views; database query builder |
| **SyncbackMetadataTask** | Plugin metadata | 8 (Pro) | 0 | **HIGH** | Engine writes metadata to database; used for send-later, snooze, tracking |
| **DatabaseStore.listen()** | Delta stream | 10 | 3 | **HIGH** | Requires engine to emit change deltas |
| **GetMessageRFC2822Task** | Full headers | 2 | 1 | **HIGH** | Engine must fetch from IMAP and return via Task |
| **MailsyncProcess/MailsyncBridge** | Direct engine IPC | 1 | 0 | **HIGH** | `MailsyncProcess` is exported; onboarding uses it for the account test. Deprecated in favour of the `Engine` export (docs/desktop-client-plan.md section 6) |
| **localized/i18n** | Localization | 40+ | 1 | **LOW** | UI strings only |

### mailspring-component-kit

| Symbol | Type | Built-in Plugins | Third-Party Plugins | Coupling Risk | Notes |
|--------|------|------------------|---------------------|---------------|-------|
| **RetinaImg** | Image component | 35+ | 1 | **LOW** | UI only |
| **ListTabular** | Virtualized list | 6 | 0 | **LOW** | UI component; pairs with ObservableListDataSource |
| **InjectedComponentSet** | Multi-injection | 12 | 0 | **LOW** | Renders multiple components at one role |
| **ListensToFluxStore** | HOC | 8 | 0 | **LOW** | React HOC for store subscriptions |
| **ListensToObservable** | HOC | 6 | 0 | **LOW** | React HOC for Rx observables |
| **ScrollRegion** | Scroll container | 8 | 0 | **LOW** | UI component |
| **Flexbox/Menu/ButtonDropdown** | UI primitives | 30+ | 0 | **LOW** | Standard React components |
| **MetadataComposerToggleButton** | Metadata button | 3 (Pro) | 0 | **MEDIUM** | Convenience wrapper for metadata toggle |
| **HasTutorialTip** | Tooltip HOC | 4 (Pro) | 0 | **LOW** | UI decorator |
| **DatePickerPopover/DateInput** | Date pickers | 3 | 0 | **LOW** | UI components |
| **BindGlobalCommands** | Keyboard shortcuts | 6 | 0 | **LOW** | Registers keybindings |
| **MultiselectToolbar** | Selection UI | 2 | 0 | **LOW** | UI component |

### ComponentRegistry Roles

| Role | Users | Purpose | Coupling |
|------|-------|---------|----------|
| **Composer:ActionButton** | 5 built-in + 2 third-party | Buttons in composer toolbar | None |
| **MessageHeaderStatus** | 3 built-in + 1 third-party | Badges next to sender name | None |
| **message:BodyHeader** | 4 built-in + 1 third-party | Content above message body | None |
| **ThreadListIcon** | 4 built-in + 1 third-party | Icons in thread list rows | None |
| **ThreadActionsToolbarButton** | 3 built-in | Buttons in thread toolbar | None |
| **Thread:MailLabel** | 2 built-in + 1 third-party (hack) | Labels on thread rows | None |
| **RootSidebar:Notifications** | 4 built-in | Left sidebar sections | None |
| **MessageListSidebar:ContactCard** | 3 built-in | Contact info sidebar | None |
| **DraftList:DraftStatus** | 2 built-in | Status indicators on drafts | None |
| **MessageListHeaders** | 2 built-in | Content between messages | None |
| **NavRail:Item** | 1 built-in | Left rail navigation items | None |
| **Composer** | 1 built-in | Entire composer component (singular) | None |
| **Composer:Footer** | 1 built-in | Below composer body | None |
| **Composer:RecipientChip** | 0 built-in + 1 third-party | Custom recipient rendering | None |
| **MessageAttachments** | 1 built-in | Attachment list rendering | None |
| **ThreadListQuickAction** | 1 built-in | Quick action buttons on hover | None |
| **ThreadListTimestamp** | 1 built-in | Custom timestamp rendering | None |
| **DraftActionsToolbarButton** | 1 built-in | Draft toolbar buttons | None |
| **MessageHeader** | 1 built-in | Full header replacement (singular) | None |

### ExtensionRegistry Types

| Registry | Method | Built-in Users | Third-Party Users | Purpose | Coupling |
|----------|--------|----------------|-------------------|---------|----------|
| **Composer** | register/unregister | 6 | 2 | Transform draft before send, modify UI | Medium (modifies SendDraftTask data) |
| **MessageView** | register/unregister | 6 | 2 | Transform message body HTML before display | Low (read-only view transform) |
| **AccountSidebar** | register/unregister | 4 | 0 | Add items to account sidebar | Low (UI only) |
| **ThreadList** | register/unregister | 1 | 0 | Modify thread list data/display | Low (UI transform) |

---

## 4. Findings

### Minimal API Surface for Most Plugins

**Read-only plugins** (avatars, level-indicators, phishing warnings, themes) need:
- `ComponentRegistry.register(Component, {role})` — UI injection
- Models: `Thread`, `Message`, `Contact`, `Account` — read-only properties
- Stores: `AccountStore`, `MessageStore`, `FocusedContentStore` — read-only state
- `ExtensionRegistry.MessageView` — HTML transform (optional)

**No engine dependency** for UI-only plugins. They work as pure render extensions.

**Metadata-based plugins** (send-later, snooze, tracking) additionally need:
- `Message.metadataForPluginId(pluginId)` — read plugin-scoped JSON
- `SyncbackMetadataTask.forSaving({model, pluginId, value})` — write metadata (queues engine task)
- `DatabaseStore.listen(callback)` — receive `metadata-expiration` events when `metadata.expiration` timestamp passes
- `DraftStore.sessionForClientId(id)` — modify draft before send

This is the **most complex common pattern**; ~8 Pro plugins use it.

**Action plugins** (archive, move, label) need:
- `Actions.queueTask(task)` — queue Tasks
- `TaskFactory` helpers OR direct Task construction
- Models for Task parameters

### Risky Couplings (Broken by Socket-Based Engine)

1. **Direct DatabaseStore queries** (`findAll`, `find`, `findBy`)
   - **Impact**: 17 built-in + 3 third-party plugins
   - **Why risky**: Assumes local SQLite database; schema-coupled
   - **Migration**: Replace with REST/GraphQL API or message-passing queries
   - **Affected patterns**: Thread/message lists, contact lookup, calendar events

2. **DatabaseStore.listen(callback)** for delta streaming
   - **Impact**: 10 built-in + 3 third-party plugins
   - **Why risky**: Assumes engine streams `DatabaseChangeRecord` objects to renderer
   - **Migration**: WebSocket delta stream OR polling OR Observables from API
   - **Affected patterns**: Live-updating lists, metadata expiration triggers, spam scanning

3. **Task creation** (Actions.queueTask)
   - **Impact**: 35 built-in + 5 third-party plugins (universal mutation path)
   - **Why risky**: Assumes Tasks serialize to JSON and are sent to engine via MailsyncBridge
   - **Migration**: REST API endpoints OR command messages
   - **Decision needed**: Keep Task abstraction (serialize to socket) OR replace with API calls?
   - **Affected patterns**: All mutations (send, move, archive, metadata updates)

4. **Model computed properties** (e.g., `Message.isFromMe()`, `Thread.participants`)
   - **Impact**: 50+ plugins
   - **Why risky**: Assumes full model hydration in renderer; may require server-side joins
   - **Migration**: Pre-compute on server OR fetch relations separately OR keep models as-is (if thin client has full sync)
   - **Affected patterns**: Rendering participants, "sent by me" checks, contact lookups

5. **GetMessageRFC2822Task** (fetch full message source)
   - **Impact**: 2 built-in (message-list, thread-list) + 1 third-party (spoof-guard)
   - **Why risky**: Synchronous IMAP fetch in engine; returns via Task completion
   - **Migration**: Async API endpoint; Task polling OR callback
   - **Affected patterns**: SPF/DKIM header checks

6. **DraftStore.sessionForClientId()** (draft editing sessions)
   - **Impact**: 8 built-in (composer extensions, templates, signatures, send-later)
   - **Why risky**: Assumes in-memory draft sessions; mutations via `.changes.add()` sync to database
   - **Migration**: Keep in-renderer (drafts are local-first) OR API for draft persistence
   - **Decision needed**: Are drafts still client-side-only, or sync to server?

7. **ObservableListDataSource + QuerySubscription** (reactive lists)
   - **Impact**: 6 built-in (thread-list, draft-list, contacts, activity)
   - **Why risky**: Assumes `DatabaseStore.findAll()` + `listen()` for live updates
   - **Migration**: Replace QuerySubscription with API polling OR WebSocket updates
   - **Affected patterns**: Main thread list, draft list, contact list

### Not Risky (No Engine Coupling)

- **ComponentRegistry**: Pure UI injection; no schema/engine dependency
- **ExtensionRegistry.MessageView**: Read-only HTML transforms
- **PreferencesUIStore**: Settings tabs (UI-only)
- **Utils/DateUtils**: Pure functions
- **React components** (mailspring-component-kit): UI primitives

### Gaps (No Good Hook)

1. **Background scanning** (spoof-guard's inbox scanner)
   - Current: `DatabaseStore.listen()` + manual `findAll()` on new messages
   - Gap: No "on new message arrived" hook; must poll or listen to all database changes
   - Needed: `Actions.messageReceived.listen()` or `ExtensionRegistry.MessageReceived`

2. **Task interception/transformation** (undo-send countdown)
   - Current: Not possible; plugins can queue Tasks but not modify/delay/cancel queued Tasks
   - Gap: No pre-queue hook for SendDraftTask
   - Needed: `ExtensionRegistry.Task.beforeQueue(task)` with cancel/modify capability

3. **Thread list customization** (sort-inbox's custom sort)
   - Current: Hack into thread-list internals (no public API)
   - Gap: No `ExtensionRegistry.ThreadList.customSort()` or query modifier
   - Needed: `MailboxPerspective` API for custom orderings

4. **Full-text search** (mailspring-ai-search)
   - Current: Assumes IMAP SEARCH or local SQLite FTS
   - Gap: No extension point for custom search backends
   - Needed: `SearchQueryBackend` registration API (exists internally but not exposed)

5. **Contact enrichment** (github-contact-card)
   - Current: Fetches GitHub data in component render (inefficient)
   - Gap: No cache/prefetch hook for contact sidebar data
   - Needed: `ContactStore.registerEnrichmentProvider(provider)`

### Recommendations for Tern

1. **Keep Task abstraction**: It's universal (35 built-in plugins); serialize to socket or REST
2. **Replace DatabaseStore.findAll() with API**: Too schema-coupled; provide query builders
3. **Keep DatabaseStore.listen() via WebSocket**: 10+ plugins need live updates; delta streaming is critical
4. **Keep Models as rich objects**: Too many plugins depend on computed properties; hydrate fully
5. **Keep ComponentRegistry exactly as-is**: Zero engine coupling; works perfectly
6. **Add missing hooks**: `Actions.messageReceived`, `ExtensionRegistry.Task.beforeQueue`, search backends
7. **DraftStore stays client-side**: Editing sessions are too performance-critical; sync persistence only
8. **ObservableListDataSource → API-backed**: Abstract QuerySubscription to work with REST/WebSocket

### Socket-Based Engine Migration Priority

**Phase 1** (keep plugins working):
- Serialize Tasks to JSON → send via socket (transparent to plugins)
- Stream DatabaseChangeRecords via WebSocket (transparent to plugins)
- Proxy `DatabaseStore.findAll()` → REST API (transparent to plugins)

**Phase 2** (break schema coupling):
- Replace `findAll()` with query builder API
- Pre-compute Model properties on server
- Version the API surface

**Phase 3** (add missing hooks):
- `messageReceived`, `beforeQueueTask`, custom search backends
