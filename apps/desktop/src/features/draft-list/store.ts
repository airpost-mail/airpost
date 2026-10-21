import BaseStore from '../../data/stores/base';
import Rx from 'rx-lite';
import * as Actions from '../../data/actions';
import { Message } from '../../data/models/message';
import OutboxStore from '../../data/stores/outbox';
import { AccountStore } from '../../data/stores/account';
import { MutableQueryResultSet } from '../../data/query/mutable-result-set';
import { MutableQuerySubscription } from '../../data/query/mutable-subscription';
import { ObservableListDataSource } from '../../components/list/observable-data-source';
import FocusedPerspectiveStore from '../../data/stores/focused-perspective';
import DatabaseStore from '../../data/database/store';
import { QueryResultSet } from '../../data/query/result-set';
import FocusedContentStore from '../../data/stores/focused-content';
import { ListTabular, ListDataSource } from '../../components/list/tabular';

/*
The preview follows the focused draft by headerMessageId, not by id: changing the From
account recreates the draft under a new id (DraftEditingSession.ensureCorrectAccount), and
FocusedContentStore drops focus when the old id is unpersisted.
*/
class DraftListStore extends BaseStore {
  constructor() {
    super();
    this.listenTo(FocusedPerspectiveStore, this._onPerspectiveChanged);
    this.listenTo(Actions.setFocus, this._onSetFocus);
    this.listenTo(Actions.composePopoutDraft, this._onPopoutDraft);
    this.listenTo(DatabaseStore, this._onDataChanged);
    this._createListDataSource();
  }

  _dataSource: ListDataSource;
  _focusedHeaderMessageId: string | null = null;
  _previewFocusHandler: (() => void) | null = null;

  dataSource = () => {
    return this._dataSource;
  };

  selectionObservable = () => {
    return Rx.Observable.fromListSelection(this);
  };

  focusedHeaderMessageId = () => {
    return this._focusedHeaderMessageId;
  };

  // Drafts a list command applies to: the selection, else the focused draft, else the
  // keyboard cursor, so Delete works without first checking a row.
  draftsForCommand = (): Message[] => {
    const selected = this._dataSource ? this._dataSource.selection.items() : [];
    if (selected.length > 0) {
      return selected as Message[];
    }
    const focused = FocusedContentStore.focused('draft') as Message | null;
    if (focused) {
      return [focused];
    }
    const cursor = FocusedContentStore.keyboardCursorEnabled()
      ? (FocusedContentStore.keyboardCursor('draft') as Message | null)
      : null;
    return cursor ? [cursor] : [];
  };

  setPreviewFocusHandler = (handler: () => void) => {
    this._previewFocusHandler = handler;
  };

  clearPreviewFocusHandler = (handler: () => void) => {
    if (this._previewFocusHandler === handler) {
      this._previewFocusHandler = null;
    }
  };

  // Returns false when no preview composer is mounted to take focus.
  focusPreview = () => {
    if (!this._focusedHeaderMessageId || !this._previewFocusHandler) {
      return false;
    }
    this._previewFocusHandler();
    return true;
  };

  // Inbound Events

  _onPerspectiveChanged = () => {
    this._createListDataSource();
    Actions.setFocus({ collection: 'draft', item: null });
  };

  _onSetFocus = ({ collection, item }: { collection: string; item: Message | null }) => {
    if (collection !== 'draft') {
      return;
    }
    const headerMessageId = item ? item.headerMessageId : null;
    if (headerMessageId !== this._focusedHeaderMessageId) {
      this._focusedHeaderMessageId = headerMessageId;
      this.trigger(this);
    }
  };

  // Editing the same draft in the preview and a popout makes the two sessions overwrite
  // each other (Foundry376/Mailspring#2158), so the preview lets go of a draft that pops out.
  _onPopoutDraft = (headerMessageId: string) => {
    if (headerMessageId && headerMessageId === this._focusedHeaderMessageId) {
      Actions.setFocus({ collection: 'draft', item: null });
    }
  };

  _onDataChanged = (change: { type: string; objectClass: string; objects: Message[] }) => {
    const headerMessageId = this._focusedHeaderMessageId;
    if (!headerMessageId || !change || change.objectClass !== 'Message') {
      return;
    }
    const touched = change.objects.filter((m) => m.headerMessageId === headerMessageId);
    if (touched.length === 0) {
      return;
    }
    // Every save of the focused draft lands here; only a new id or a removal needs a lookup.
    const focusedId = FocusedContentStore.focusedId('draft');
    if (change.type === 'persist' && touched.every((m) => m.draft && m.id === focusedId)) {
      return;
    }
    this._resolveFocusedDraft(headerMessageId);
  };

  _resolveFocusedDraft = async (headerMessageId: string) => {
    const perspective = FocusedPerspectiveStore.current();
    const query = DatabaseStore.findBy<Message>(Message, { headerMessageId, draft: true });
    if (perspective.accountIds.length < AccountStore.accounts().length) {
      query.where({ accountId: perspective.accountIds });
    }
    const draft = await query;
    if (headerMessageId !== this._focusedHeaderMessageId) {
      return;
    }
    if (!draft) {
      Actions.setFocus({ collection: 'draft', item: null });
    } else if (FocusedContentStore.focusedId('draft') !== draft.id) {
      Actions.setFocus({ collection: 'draft', item: draft });
    }
  };

  // Internal

  _createListDataSource = () => {
    const mailboxPerspective = FocusedPerspectiveStore.current();

    if (this._dataSource) {
      this._dataSource.cleanup();
      this._dataSource = null;
    }

    if ((mailboxPerspective as any).drafts) {
      const query = DatabaseStore.findAll<Message>(Message)
        .include(Message.attributes.body)
        .order(Message.attributes.date.descending())
        .where({ draft: true })
        .page(0, 1);

      // Adding a "account_id IN (a,b,c)" clause to our query can result in a full
      // table scan. Don't add the where clause if we know we want results from all.
      if (mailboxPerspective.accountIds.length < AccountStore.accounts().length) {
        query.where({ accountId: mailboxPerspective.accountIds });
      }

      const subscription = new MutableQuerySubscription(query, { emitResultSet: true });
      const $resultSet = Rx.Observable.combineLatest(
        Rx.Observable.fromNamedQuerySubscription('draft-list', subscription),
        Rx.Observable.fromStore(OutboxStore),
        (resultSet, outboxStore) => {
          if (!(resultSet instanceof QueryResultSet)) {
            throw 'Set emitResultSet=true and did not receive a QueryResultSet';
          }
          // Generate a new result set that includes additional information on
          // the draft objects. This is similar to what we do in the thread-list,
          // where we set thread.__messages to the message array.
          const resultSetWithTasks = new MutableQueryResultSet(resultSet);

          // TODO BG modelWithId: task.headerMessageId does not work
          mailboxPerspective.accountIds.forEach((aid) => {
            outboxStore.itemsForAccount(aid).forEach((task) => {
              let draft = resultSet.modelWithId(task.headerMessageId) as any;
              if (draft) {
                draft = draft.clone();
                draft.uploadTaskId = task.id;
                draft.uploadProgress = task.progress;
                resultSetWithTasks.updateModel(draft);
              }
            });
          });

          return resultSetWithTasks.immutableClone();
        }
      );

      this._dataSource = new ObservableListDataSource($resultSet, subscription.replaceRange);
    } else {
      this._dataSource = new ListTabular.DataSource.Empty();
    }

    this.trigger(this);
  };
}

export default new DraftListStore();
