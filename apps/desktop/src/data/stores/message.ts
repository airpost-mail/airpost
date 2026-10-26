import BaseStore from './base';
import * as Actions from '../actions';
import { Message } from '../models/message';
import { Thread } from '../models/thread';
import DatabaseStore from '../database/store';
import { TaskFactory } from '../tasks/factory';
import FocusedPerspectiveStore from './focused-perspective';
import FocusedContentStore from './focused-content';
import * as ExtensionRegistry from '../../packages/extension-registry';
import electron from 'electron';
import { MessageViewExtension } from '../../packages/message-view-extension';
import { CurrentWindow } from '../../platform/app-api';
import { QuerySubscription } from '../query/subscription';
import QuerySubscriptionPool from '../query/subscription-pool';

const FolderNamesHiddenByDefault = ['spam', 'trash'];

// A thread opens with at most this many unread messages expanded, the newest; "expand all"
// opens the rest. Each expanded message mounts an EmailFrame.
export const MAX_DEFAULT_EXPANDED_UNREAD = 3;

let threadSubscriptions = 0;

const byDate = (a: Message, b: Message) =>
  (a.date ? a.date.getTime() : 0) - (b.date ? b.date.getTime() : 0);

class _MessageStore extends BaseStore {
  FolderNamesHiddenByDefault = FolderNamesHiddenByDefault;

  _showingHiddenItems = false;
  _items: Message[] = [];
  _thread: Thread | null = null;
  _itemsExpanded: { [messageId: string]: 'explicit' | 'default' } = {};
  _itemsLoading = false;
  _lastMarkedAsReadThreadId?: string | null;
  _onFocusChangedTimer?: NodeJS.Timeout | null;
  // The open thread's messages and the thread itself, kept current by two QuerySubscriptions
  // (live queries on the engine); ends them.
  _unsubscribeThread: (() => void) | null = null;
  // The open thread's messages have come at least once.
  _messagesArrived = false;
  // The ids the last result of the thread's messages held.
  _lastMessageIds = new Set<string>();
  // Ids whose bodies are being read, so overlapping loads don't ask for them again.
  _bodiesLoading = new Set<string>();
  _draftToFocus: string | null = null;

  constructor() {
    super();
    this._setStoreDefaults();
    this._registerListeners();
  }

  //########## PUBLIC #####################################################

  items() {
    if (this._showingHiddenItems) return this._items;

    const viewing = FocusedPerspectiveStore.current().categoriesSharedRole();
    const viewingHiddenCategory = viewing !== null && FolderNamesHiddenByDefault.includes(viewing);

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

  // Expands a draft of the open thread, for the message list to focus its composer
  // (takeDraftToFocus). False when the thread has no such draft.
  focusDraft(headerMessageId: string) {
    const item = this._items.find((i) => i.draft && i.headerMessageId === headerMessageId);
    if (!item) return false;
    if (!this._itemsExpanded[item.id]) this._expandItem(item);
    this._draftToFocus = headerMessageId;
    this.trigger();
    return true;
  }

  takeDraftToFocus() {
    const headerMessageId = this._draftToFocus;
    this._draftToFocus = null;
    return headerMessageId;
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
    const MessageBodyProcessor = require('../../mail/body/processor').default;
    MessageBodyProcessor.resetCache();
  }

  //########## PRIVATE ####################################################

  _setStoreDefaults() {
    this._endThreadSubscriptions();
    this._items = [];
    this._itemsExpanded = {};
    this._draftToFocus = null;
    this._bodiesLoading = new Set();
    this._itemsLoading = false;
    this._showingHiddenItems = false;
    this._thread = null;
  }

  _registerListeners() {
    this.listenTo(ExtensionRegistry.MessageView, this._onExtensionsChanged);
    this.listenTo(FocusedContentStore, this._onFocusChanged);
    this.listenTo(FocusedPerspectiveStore, this._onPerspectiveChanged);
    this.listenTo(Actions.toggleMessageIdExpanded, this._onToggleMessageIdExpanded);
    this.listenTo(Actions.toggleAllMessagesExpanded, this._onToggleAllMessagesExpanded);
    this.listenTo(Actions.toggleHiddenMessages, this._onToggleHiddenMessages);
    this.listenTo(Actions.popoutThread, this._onPopoutThread);
    this.listenTo(Actions.focusThreadMainWindow, this._onFocusThreadMainWindow);
  }

  _onPerspectiveChanged() {
    return this.trigger();
  }

  // The open thread's messages, each time they change. Bodies come separately
  // (_loadExpandedBodies), so a message keeps the body it showed; a live query's rows never
  // carry one, so a changed draft or a message whose body hadn't downloaded reads its body
  // again (_refreshBodies).
  _onMessages(threadId: string, messages: Message[]) {
    if (threadId !== this.threadId()) return;
    const shown = new Map(this._items.map((m) => [m.id, m]));
    const refresh: string[] = [];
    for (const message of messages) {
      const old = shown.get(message.id);
      if (!old || message.body !== undefined || old.body === undefined) continue;
      message.body = old.body;
      if (message !== old && (old.draft || old.body === null) && this._itemsExpanded[message.id]) {
        refresh.push(message.id);
      }
    }
    const next = this._sortItemsForDisplay(
      this._oneItemPerDraft(
        messages.filter((m) => !m.isHidden()),
        shown,
        this._lastMessageIds
      ).sort(byDate)
    );
    this._lastMessageIds = new Set(messages.map((m) => m.id));

    if (!this._messagesArrived) {
      this._messagesArrived = true;
      this._items = next;
      this._expandItemsToDefault();
      this._loadExpandedBodies(() => {
        if (threadId !== this.threadId()) return;
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
      return;
    }

    this._items = next;
    this._expandNewItems(new Set(shown.keys()));
    this._fetchExpandedAttachments(this._items);
    this.trigger();
    this._loadExpandedBodies(() => this.trigger());
    if (refresh.length) this._refreshBodies(refresh);
  }

  // The open thread itself changed.
  _onThread(threadId: string, thread: Thread | null) {
    if (!thread || threadId !== this.threadId() || thread === this._thread) return;
    this._thread = thread;
    this.trigger();
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

  // One item per draft: two would mount two composers on the same editing session, which
  // feed each other's changes back without end. A draft that arrived under another id
  // replaces the one shown; otherwise the shown one stays.
  _oneItemPerDraft(messages: Message[], shown: Map<string, Message>, before: Set<string>) {
    const rank = (m: Message) => (!before.has(m.id) ? 2 : shown.has(m.id) ? 1 : 0);
    const byHeader = new Map<string, Message>();
    for (const m of messages) {
      if (!m.draft || !m.headerMessageId) continue;
      const other = byHeader.get(m.headerMessageId);
      if (!other || rank(m) > rank(other)) byHeader.set(m.headerMessageId, m);
    }
    return messages.filter(
      (m) => !m.draft || !m.headerMessageId || byHeader.get(m.headerMessageId) === m
    );
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
    const focused = FocusedContentStore.focused('thread');
    if (focused === null) {
      this._lastMarkedAsReadThreadId = null;
    }

    // if we already match the desired state, no need to trigger
    if (this.threadId() === (focused ? focused.id : undefined)) return;

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
    CurrentWindow.setTitle(title);
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
        const thread = this._thread;
        if (markAsReadId !== this.threadId() || !thread || !thread.unread) return;
        Actions.queueTask(
          TaskFactory.taskForInvertingUnread({
            threads: [thread],
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

  // Follows the open thread's messages (without bodies) and the thread; the bodies of the
  // ones shown expanded load once they come. Replaces the subscriptions of a thread opened
  // before.
  _fetchFromCache() {
    this._endThreadSubscriptions();
    this._messagesArrived = false;
    this._lastMessageIds = new Set();
    if (!this._thread) return;

    const threadId = this._thread.id;
    const key = `message-store-${++threadSubscriptions}`;
    const messages = new QuerySubscription<Message>(
      DatabaseStore.findAll<Message>(Message).where({ threadId })
    );
    const thread = new QuerySubscription<Thread>(DatabaseStore.find<Thread>(Thread, threadId));
    // Reads that fail keep what is shown (QuerySubscription logs them); the first one still
    // ends the loading state.
    const failed = messages._onReadFailed.bind(messages);
    messages._onReadFailed = (err: Error) => {
      failed(err);
      if (threadId !== this.threadId() || this._messagesArrived) return;
      this._itemsLoading = false;
      this.trigger(this);
    };
    const offMessages = QuerySubscriptionPool.addPrivateSubscription(
      `${key}-messages`,
      messages,
      (result: Message[]) => this._onMessages(threadId, result)
    );
    const offThread = QuerySubscriptionPool.addPrivateSubscription(
      `${key}-thread`,
      thread,
      (result: Thread | null) => this._onThread(threadId, result)
    );
    this._unsubscribeThread = () => {
      offMessages();
      offThread();
    };
  }

  _endThreadSubscriptions() {
    if (!this._unsubscribeThread) return;
    this._unsubscribeThread();
    this._unsubscribeThread = null;
  }

  // Reads the bodies of `ids` again and shows them, without asking the engine to download the
  // ones it hasn't (the message's own delta says when it has).
  _refreshBodies(ids: string[]) {
    const threadId = this.threadId();
    DatabaseStore.findAll<Message>(Message, { id: ids })
      .include(Message.attributes.body)
      .then(
        (loaded) => {
          if (threadId !== this.threadId()) return;
          const bodies = new Map(loaded.map((m) => [m.id, m.body]));
          let changed = false;
          this._items = this._items.map((item) => {
            if (!bodies.has(item.id) || bodies.get(item.id) === item.body) return item;
            changed = true;
            const withBody = new Message(item);
            withBody.body = bodies.get(item.id);
            return withBody;
          });
          if (changed) this.trigger();
        },
        (err) => console.warn(`MessageStore: could not read message bodies: ${err}`)
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
    let index: number;
    let item: Message | null;
    const itemsInReplyTo: Message[] = [];
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
        if (item && item.replyToHeaderMessageId === other.headerMessageId) {
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
