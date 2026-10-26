import BaseStore from '../../data/stores/base';

import Rx from 'rx-lite';
import * as Actions from '../../data/actions';
import { Thread } from '../../data/models/thread';
import { QueryResultSet } from '../../data/query/result-set';
import { MutableQuerySubscription } from '../../data/query/mutable-subscription';
import WorkspaceStore from '../../data/stores/workspace';
import FocusedContentStore from '../../data/stores/focused-content';
import FocusedPerspectiveStore from '../../data/stores/focused-perspective';
import { MailboxPerspective } from '../../data/mailbox-perspective';
import { ListTabular, ListDataSource } from '../../components/list/tabular';
import ThreadListDataSource from './data-source';

class ThreadListStore extends BaseStore {
  // Set by createListDataSource(), which the constructor calls.
  _dataSource!: ListDataSource;
  _dataSourceUnlisten?: () => void;
  _perspectiveKey: string | null = null;

  constructor() {
    super();
    this.listenTo(FocusedPerspectiveStore, this._onPerspectiveChanged);
    this.createListDataSource();
  }

  dataSource = () => {
    return this._dataSource;
  };

  createListDataSource = () => {
    if (typeof this._dataSourceUnlisten === 'function') {
      this._dataSourceUnlisten();
    }

    if (this._dataSource) {
      this._dataSource.cleanup();
    }

    this._perspectiveKey = ThreadListStore.perspectiveKey(FocusedPerspectiveStore.current());
    const threadsSubscription = FocusedPerspectiveStore.current().threads();
    // Perspectives return mutable subscriptions: the list pages through them with replaceRange.
    if (threadsSubscription instanceof MutableQuerySubscription) {
      this._dataSource = new ThreadListDataSource(threadsSubscription);
      this._dataSourceUnlisten = this._dataSource.listen(this._onDataChanged, this);
    } else {
      this._dataSource = new ListTabular.DataSource.Empty();
    }

    this.trigger(this);
    Actions.setFocus({ collection: 'thread', item: null });
  };

  selectionObservable = () => {
    return Rx.Observable.fromListSelection<Thread>(this);
  };

  // Inbound Events

  // What the thread query of a category perspective depends on. A renamed folder or label
  // keeps its id, so FocusedPerspectiveStore swaps in a perspective with the new path and
  // the same key: the list, its scroll position and the open thread stay. null for other
  // perspectives, which are always rebuilt.
  static perspectiveKey(perspective: MailboxPerspective | null): string | null {
    const json = perspective && (perspective.toJSON() as { serializedCategories?: string });
    if (!json || !json.serializedCategories) {
      return null;
    }
    return JSON.stringify([
      perspective.constructor.name,
      perspective.accountIds,
      perspective.categories().map((c) => [c.id, c.role]),
    ]);
  }

  _onPerspectiveChanged = () => {
    const key = ThreadListStore.perspectiveKey(FocusedPerspectiveStore.current());
    if (key !== null && key === this._perspectiveKey) {
      this.trigger(this);
      return;
    }
    this.createListDataSource();
  };

  _onDataChanged = ({
    previous,
    next,
  }: { previous?: QueryResultSet<Thread>; next?: QueryResultSet<Thread> } = {}) => {
    // This code keeps the focus and keyboard cursor in sync with the thread list.
    // When the thread list changes, it looks to see if the focused thread is gone,
    // or no longer matches the query criteria and advances the focus to the next
    // thread.

    // This means that removing a thread from view in any way causes selection
    // to advance to the adjacent thread. Nice and declarative.
    if (previous && next) {
      const focused = FocusedContentStore.focused('thread');
      const keyboard = FocusedContentStore.keyboardCursor('thread') as Thread | null;
      const viewModeAutofocuses =
        FocusedPerspectiveStore.isShowingMail() &&
        (WorkspaceStore.layoutMode() === 'split' || WorkspaceStore.topSheet().root === true);

      const nextQ = next.query();
      const matchers = nextQ && nextQ.matchers();

      const focusedIndex = focused ? previous.offsetOfId(focused.id) : -1;
      const keyboardIndex = keyboard ? previous.offsetOfId(keyboard.id) : -1;

      const nextItemFromIndex = (i: number) => {
        let nextIndex;
        if (
          i > 0 &&
          ((next.modelAtOffset(i - 1) && next.modelAtOffset(i - 1).unread) || i >= next.count())
        ) {
          nextIndex = i - 1;
        } else {
          nextIndex = i;
        }

        // May return null if no thread is loaded at the next index
        return next.modelAtOffset(nextIndex);
      };

      // A live query's set is the engine's answer: a thread left the view when the window
      // held it and no longer does (a scroll moves the window, which isn't that). Otherwise
      // the query's matchers decide.
      const notInSet = function (model: Thread) {
        if (next._live) {
          return next.liveRemoved(previous, model.id);
        }
        if (matchers) {
          return model.matches(matchers) === false;
        } else {
          return next.offsetOfId(model.id) === -1;
        }
      };

      if (viewModeAutofocuses && focused && notInSet(focused)) {
        Actions.setFocus({ collection: 'thread', item: nextItemFromIndex(focusedIndex) });
      }

      if (keyboard && notInSet(keyboard)) {
        Actions.setCursorPosition({
          collection: 'thread',
          item: nextItemFromIndex(keyboardIndex),
        });
      }
    }
  };
}

export default new ThreadListStore();
