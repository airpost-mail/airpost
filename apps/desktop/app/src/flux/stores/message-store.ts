import MailspringStore from 'mailspring-store';
import * as Actions from '../actions';
import { Message } from '../models/message';
import { Thread } from '../models/thread';
import DatabaseStore from './database-store';
import { TaskFactory } from '../tasks/task-factory';
import FocusedPerspectiveStore from './focused-perspective-store';
import FocusedContentStore from './focused-content-store';
import * as ExtensionRegistry from '../../registries/extension-registry';
import electron from 'electron';
import { MessageViewExtension } from 'mailspring-exports';
import { DatabaseChangeRecord } from './database-change-record';
import EngineReads from './engine-reads';

const FolderNamesHiddenByDefault = ['spam', 'trash'];

// A thread opens with at most this many unread messages expanded, the newest; "expand all"
// opens the rest. Each expanded message mounts an EmailFrame.
export const MAX_DEFAULT_EXPANDED_UNREAD = 3;

type MessageChange = { type: string; objects: Message[] };

const byDate = (a: Message, b: Message) =>
  (a.date ? a.date.getTime() : 0) - (b.date ? b.date.getTime() : 0);

class _MessageStore extends MailspringStore {
  FolderNamesHiddenByDefault = FolderNamesHiddenByDefault;

  _showingHiddenItems = false;
  _items?: Message[];
  _thread?: Thread;
  _itemsExpanded: { [messageId: string]: 'explicit' | 'default' };
  _itemsLoading: boolean;
  _lastMarkedAsReadThreadId?: string;
  _onFocusChangedTimer?: NodeJS.Timeout;
  // Message changes for the thread that arrive while its messages are being read, applied on
  // top of the result; null when no read is out.
  _changesWhileLoading: MessageChange[] = null;
  // Ids whose bodies are being read, so overlapping loads don't ask for them again.
  _bodiesLoading = new Set<string>();

  constructor() {
    super();
    this._setStoreDefaults();
    this._registerListeners();
  }

  //########## PUBLIC #####################################################

  items() {
    if (this._showingHiddenItems) return this._items;

    const viewing = FocusedPerspectiveStore.current().categoriesSharedRole();
    const viewingHiddenCategory = FolderNamesHiddenByDefault.includes(viewing);

    // A message can have copies in several folders. It is hidden only when every copy is
    // in spam or trash; when viewing spam or trash, any copy there is enough to show it.
    return this._items.filter((item) => {
      const roles = item.categories().map((c) => c.role);
      const anyHidden = roles.some((r) => FolderNamesHiddenByDefault.includes(r));
      const allHidden =
        roles.length > 0 && roles.every((r) => FolderNamesHiddenByDefault.includes(r));
      return viewingHiddenCategory ? anyHidden || item.draft : !allHidden;
    });
  }

  threadId() {
    return this._thread ? this._thread.id : undefined;
  }

  thread() {
    return this._thread;
  }

  itemsExpandedState() {
    // ensure that we're always serving up immutable objects.
    // this.state == nextState is always true if we modify objects in place.
    // A message whose body isn't loaded yet shows collapsed until it is: MessageItemBody reads
    // the body when it mounts.
    const state = Object.assign({}, this._itemsExpanded);
    for (const item of this._items) {
      if (item.body === undefined) delete state[item.id];
    }
    return state;
  }

  hasCollapsedItems() {
    return Object.keys(this._itemsExpanded).length < this._items.length;
  }

  numberOfHiddenItems() {
    return this._items.length - this.items().length;
  }

  itemIds() {
    return this._items.map((i) => i.id);
  }

  itemsLoading() {
    return this._itemsLoading;
  }

  /*
  Message Store Extensions
  */

  // Public: Returns the extensions registered with the MessageStore.
  extensions(): (typeof MessageViewExtension)[] {
    return ExtensionRegistry.MessageView.extensions();
  }

  _onExtensionsChanged(role: string) {
    const MessageBodyProcessor = require('./message-body-processor').default;
    MessageBodyProcessor.resetCache();
  }

  //########## PRIVATE ####################################################

  _setStoreDefaults() {
    this._items = [];
    this._itemsExpanded = {};
    this._bodiesLoading = new Set();
    this._itemsLoading = false;
    this._showingHiddenItems = false;
    this._thread = null;
  }

  _registerListeners() {
    this.listenTo(ExtensionRegistry.MessageView, this._onExtensionsChanged);
    this.listenTo(DatabaseStore, this._onDataChanged);
    this.listenTo(FocusedContentStore, this._onFocusChanged);
    this.listenTo(FocusedPerspectiveStore, this._onPerspectiveChanged);
    this.listenTo(Actions.toggleMessageIdExpanded, this._onToggleMessageIdExpanded);
    this.listenTo(Actions.toggleAllMessagesExpanded, this._onToggleAllMessagesExpanded);
    this.listenTo(Actions.toggleHiddenMessages, this._onToggleHiddenMessages);
    this.listenTo(Actions.popoutThread, this._onPopoutThread);
    this.listenTo(Actions.focusThreadMainWindow, this._onFocusThreadMainWindow);
    // The deltas sent while the window had no engine connection are gone.
    EngineReads.on('resync', () => this._fetchFromCache());
  }

  _onPerspectiveChanged() {
    return this.trigger();
  }

  // Message deltas patch the loaded messages, keeping the bodies they don't carry: the engine
  // sends a message's body only with the save that set it.
  _onDataChanged(change: DatabaseChangeRecord<any>) {
    if (!this._thread) return;

    if (change.objectClass === Message.name) {
      const loaded = new Set(this._items.map((m) => m.id));
      const objects = (change.objects as Message[]).filter(
        (m) => m.threadId === this._thread.id || loaded.has(m.id)
      );
      if (objects.length === 0) return;
      if (this._changesWhileLoading) {
        this._changesWhileLoading.push({ type: change.type, objects });
        return;
      }
      this._items = this._patchedItems(this._items, { type: change.type, objects });
      this._expandNewItems(loaded);
      this._fetchExpandedAttachments(this._items);
      this.trigger();
      this._loadExpandedBodies(() => this.trigger());
    }

    if (change.objectClass === Thread.name) {
      const updatedThread = change.objects.find((t) => t.id === this._thread.id);
      if (updatedThread) {
        this._thread = updatedThread;
        this.trigger();
      }
    }
  }

  // A message that arrives unread, a new draft and a new last message open expanded; the
  // others keep the state they had.
  _expandNewItems(before: Set<string>) {
    const visibleItems = this.items();
    visibleItems.forEach((item, idx) => {
      if (before.has(item.id)) return;
      if (item.unread || item.draft || idx === visibleItems.length - 1) {
        this._itemsExpanded[item.id] = 'default';
      }
    });
  }

  _patchedItems(items: Message[], { type, objects }: MessageChange) {
    const byId = new Map(items.map((m) => [m.id, m]));
    for (const message of objects) {
      if (type === 'unpersist' || message.threadId !== this._thread.id) {
        byId.delete(message.id);
        continue;
      }
      const old = byId.get(message.id);
      if (old && message.body === undefined) message.body = old.body;
      byId.set(message.id, message);
    }
    const next = [...byId.values()].filter((m) => !m.isHidden()).sort(byDate);
    return this._sortItemsForDisplay(next);
  }

  _onFocusChanged(change: { impactsCollection: (collection: string) => boolean }) {
    if (!change.impactsCollection('thread')) return;

    // This implements a debounce that fires on the leading and trailing edge.
    //
    // If we haven't changed focus in the last 100ms, do it immediately. This means
    // there is no delay when moving to the next thread, deselecting a thread, etc.
    //
    // If we have changed focus in the last 100ms, wait for focus changes to
    // stop arriving for 100msec before applying. This means that flying
    // through threads doesn't cause is to make a zillion queries for messages.
    //
    if (!this._onFocusChangedTimer) {
      this._onApplyFocusChange();
    } else {
      clearTimeout(this._onFocusChangedTimer);
    }

    this._onFocusChangedTimer = setTimeout(() => {
      this._onFocusChangedTimer = null;
      this._onApplyFocusChange();
    }, 100);
  }

  _onApplyFocusChange() {
    const focused = FocusedContentStore.focused('thread') as Thread;
    if (focused === null) {
      this._lastMarkedAsReadThreadId = null;
    }

    // if we already match the desired state, no need to trigger
    if (this.threadId() === (focused || {}).id) return;

    this._thread = focused;
    this._items = [];
    this._itemsLoading = true;
    this._showingHiddenItems = false;
    this._itemsExpanded = {};
    this._bodiesLoading = new Set();
    this.trigger();

    this._setWindowTitle();

    this._fetchFromCache();
  }

  _setWindowTitle() {
    const title = 'Tern' + (this._thread ? ' · ' + this._thread.subject : '');
    require('@electron/remote').getCurrentWindow().setTitle(title);
  }

  _markAsRead() {
    // Mark the thread as read if necessary. Make sure it's still the
    // current thread after the timeout.
    //
    // Override canBeUndone to return false so that we don't see undo
    // prompts (since this is a passive action vs. a user-triggered
    // action.)
    if (!this._thread) return;
    if (this._lastMarkedAsReadThreadId === this._thread.id) return;
    this._lastMarkedAsReadThreadId = this._thread.id;

    if (this._thread.unread) {
      const markAsReadDelay = AppEnv.config.get('core.reading.markAsReadDelay');
      const markAsReadId = this._thread.id;
      if (markAsReadDelay < 0) return;

      setTimeout(() => {
        if (markAsReadId !== this.threadId() || !this._thread.unread) return;
        Actions.queueTask(
          TaskFactory.taskForInvertingUnread({
            threads: [this._thread],
            source: 'Thread Selected',
            canBeUndone: false,
          })
        );
      }, markAsReadDelay);
    }
  }

  _onToggleAllMessagesExpanded() {
    if (this.hasCollapsedItems()) {
      this._items.forEach((i) => this._expandItem(i));
      this._loadExpandedBodies(() => this.trigger());
    } else {
      // Do not collapse the latest message, i.e. the last one
      this._items.slice(0, -1).forEach((i) => this._collapseItem(i));
    }
    this.trigger();
  }

  _onToggleHiddenMessages() {
    this._showingHiddenItems = !this._showingHiddenItems;
    this._expandItemsToDefault();
    this._fetchExpandedAttachments(this._items);
    this.trigger();
    this._loadExpandedBodies(() => this.trigger());
  }

  _onToggleMessageIdExpanded(id: string) {
    const item = this._items.find((i) => i.id === id);
    if (!item) return;

    if (this._itemsExpanded[id]) {
      this._collapseItem(item);
    } else {
      this._expandItem(item);
      this._loadExpandedBodies(() => this.trigger());
    }

    this.trigger();
  }

  // Callers load the bodies once after expanding (_loadExpandedBodies).
  _expandItem(item: Message) {
    this._itemsExpanded[item.id] = 'explicit';
    this._fetchExpandedAttachments([item]);
  }

  _collapseItem(item: Message) {
    delete this._itemsExpanded[item.id];
  }

  // Reads the thread's messages without bodies, then the bodies of the ones shown expanded.
  _fetchFromCache() {
    if (!this._thread) return;

    const loadedThreadId = this._thread.id;
    const changes: MessageChange[] = [];
    this._changesWhileLoading = changes;

    const query = DatabaseStore.findAll<Message>(Message);
    query.where({ threadId: loadedThreadId });

    return query.then(
      (items) => {
        // Check to make sure that our thread is still the thread we were
        // loading items for. Necessary because this takes a while.
        if (loadedThreadId !== this.threadId() || this._changesWhileLoading !== changes) return;
        this._changesWhileLoading = null;

        // A re-read of the open thread (after a reconnect) keeps the bodies it showed; a draft's
        // may have changed meanwhile.
        const shown = new Map(this._items.map((m) => [m.id, m]));
        for (const item of items) {
          const old = shown.get(item.id);
          if (old && !old.draft && item.body === undefined) item.body = old.body;
        }
        let next = this._sortItemsForDisplay(items.filter((m) => !m.isHidden()));
        for (const change of changes) next = this._patchedItems(next, change);
        this._items = next;
        this._expandItemsToDefault();

        this._loadExpandedBodies(() => {
          // Download the attachments on expanded messages.
          this._fetchExpandedAttachments(this._items);

          // Normally, we would trigger often and let the view's
          // shouldComponentUpdate decide whether to re-render, but if we
          // know we're not ready, don't even bother.  Trigger once at start
          // and once when ready. Many third-party stores will observe
          // MessageStore and they'll be stupid and re-render constantly.
          this._itemsLoading = false;
          this._markAsRead();
          this.trigger(this);
        });
      },
      (err) => {
        if (loadedThreadId !== this.threadId() || this._changesWhileLoading !== changes) return;
        this._changesWhileLoading = null;
        console.warn(`MessageStore: could not read the messages of ${loadedThreadId}: ${err}`);
        this._itemsLoading = false;
        this.trigger(this);
      }
    );
  }

  // Loads the bodies of the expanded messages that have none loaded, asks the engine for the
  // ones it hasn't downloaded, then runs `done` (at once when nothing was missing).
  _loadExpandedBodies(done: () => void) {
    const threadId = this.threadId();
    const missing = this._items.filter(
      (m) => this._itemsExpanded[m.id] && m.body === undefined && !this._bodiesLoading.has(m.id)
    );
    if (missing.length === 0) {
      done();
      return;
    }
    const ids = missing.map((m) => m.id);
    const inFlight = this._bodiesLoading;
    ids.forEach((id) => inFlight.add(id));
    DatabaseStore.findAll<Message>(Message, { id: ids })
      .include(Message.attributes.body)
      .then(
        (loaded) => {
          ids.forEach((id) => inFlight.delete(id));
          if (threadId !== this.threadId()) return;
          const bodies = new Map(loaded.map((m) => [m.id, m.body]));
          this._items = this._items.map((item) => {
            if (item.body !== undefined || !bodies.has(item.id)) return item;
            const withBody = new Message(item);
            withBody.body = bodies.get(item.id);
            return withBody;
          });
          this._fetchMissingBodies(this._items.filter((m) => bodies.has(m.id)));
          done();
        },
        (err) => {
          ids.forEach((id) => inFlight.delete(id));
          if (threadId !== this.threadId()) return;
          console.warn(`MessageStore: could not read message bodies: ${err}`);
          done();
        }
      );
  }

  _fetchMissingBodies(items: Message[]) {
    const missing = items.filter((i) => i.body === null);
    if (missing.length > 0) {
      return Actions.fetchBodies(missing);
    }
  }

  _fetchExpandedAttachments(items: Message[]) {
    for (const item of items) {
      if (!this._itemsExpanded[item.id]) continue;
      item.files.map((file) => Actions.fetchFile(file));
    }
  }

  // Expand the newest unread messages (MAX_DEFAULT_EXPANDED_UNREAD of them), the last draft,
  // and the last message. Expanded messages stay expanded.
  _expandItemsToDefault() {
    const visibleItems = this.items();
    let lastDraftIdx = -1;

    visibleItems.forEach((item, idx) => {
      if (item.draft) {
        lastDraftIdx = idx;
      }
    });

    let unreadLeft = MAX_DEFAULT_EXPANDED_UNREAD;
    for (let idx = visibleItems.length - 1; idx >= 0; idx--) {
      const item = visibleItems[idx];
      const expandUnread = item.unread && unreadLeft > 0;
      if (item.unread) unreadLeft -= 1;
      if (expandUnread || idx === lastDraftIdx || idx === visibleItems.length - 1) {
        this._itemsExpanded[item.id] = 'default';
      }
    }
  }

  _sortItemsForDisplay(items: Message[]) {
    // Re-sort items in the list so that drafts appear after the message that
    // they are in reply to, when possible. First, identify all the drafts
    // with a replyToHeaderMessageId and remove them
    let index, item;
    const itemsInReplyTo = [];
    for (index = items.length - 1; index >= 0; index--) {
      item = items[index];
      if (item.draft && item.replyToHeaderMessageId) {
        itemsInReplyTo.push(item);
        items.splice(index, 1);
      }
    }

    // For each item with the reply header, re-inset it into the list after
    // the message which it was in reply to. If we can't find it, put it at the end.
    for (item of itemsInReplyTo) {
      for (index = 0; index < items.length; index++) {
        const other = items[index];
        if (item.replyToHeaderMessageId === other.headerMessageId) {
          items.splice(index + 1, 0, item);
          item = null;
          break;
        }
      }
      if (item) {
        items.push(item);
      }
    }

    return items;
  }

  _onPopoutThread(thread: Thread) {
    return AppEnv.newWindow({
      title: false, // MessageList already displays the thread subject
      hidden: false,
      windowKey: `thread-${thread.id}`,
      windowType: 'thread-popout',
      windowProps: {
        threadId: thread.id,
        perspectiveJSON: FocusedPerspectiveStore.current().toJSON(),
      },
    });
  }

  _onFocusThreadMainWindow(thread: Thread) {
    if (AppEnv.isMainWindow()) {
      FocusedPerspectiveStore.showMail();
      Actions.setFocus({ collection: 'thread', item: thread });
      return AppEnv.focus();
    }
  }
}

export const MessageStore = new _MessageStore();
