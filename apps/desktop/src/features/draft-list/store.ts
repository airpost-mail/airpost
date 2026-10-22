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
import ModelQuery from '../../data/query/query';
import { SendDraftTask } from '../../data/tasks/send-draft';
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

  // Made by _createListDataSource, which the constructor calls.
  _dataSource!: ListDataSource;
  _focusedHeaderMessageId: string | null = null;
  _previewFocusHandler: (() => void) | null = null;

  dataSource = () => {
    return this._dataSource;
  };

  selectionObservable = () => {
    return Rx.Observable.fromListSelection<Message>(this);
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
    }

    if ((mailboxPerspective as any).drafts) {
      const { accountIds } = mailboxPerspective;
      const drafts = () => {
        const query = DatabaseStore.findAll<Message>(Message)
          .include(Message.attributes.body)
          .order(Message.attributes.date.descending())
          .where({ draft: true });
        // Adding a "account_id IN (a,b,c)" clause to our query can result in a full
        // table scan. Don't add the where clause if we know we want results from all.
        if (accountIds.length < AccountStore.accounts().length) {
          query.where({ accountId: accountIds });
        }
        return query;
      };

      // The list is the pending sends, then the other drafts by date. The pending ones are
      // left out of the paged query so its offsets stay the list's, shifted by their count.
      let pinnedIds: string[] = [];
      let retained = { start: 0, end: 1 };
      const subscription = new MutableQuerySubscription(drafts().page(0, 1), {
        emitResultSet: true,
      });
      const replaceListQuery = () => {
        const query = drafts();
        if (pinnedIds.length > 0) {
          query.where(Message.attributes.id.notIn(pinnedIds));
        }
        const shift = pinnedIds.length;
        subscription.replaceQuery(
          query.page(Math.max(0, retained.start - shift), Math.max(0, retained.end - shift))
        );
      };

      const $pending = Rx.Observable.combineLatest(
        Rx.Observable.fromQuery<Message[]>(
          drafts().where(Message.attributes.pluginMetadata.contains(SEND_LATER))
        ),
        Rx.Observable.fromStore(OutboxStore),
        (withSendLater: Message[], outboxStore: typeof OutboxStore) => {
          const tasks = accountIds
            .flatMap((aid) => outboxStore.itemsForAccount(aid))
            .filter((t): t is SendDraftTask => t instanceof SendDraftTask);
          return { drafts: pendingSends(withSendLater, tasks), tasks };
        }
      ).do(({ drafts: pending }) => {
        const ids = pending.map((d) => d.id).sort();
        if (ids.join(',') !== pinnedIds.join(',')) {
          pinnedIds = ids;
          replaceListQuery();
        }
      });

      const selectionQuery = drafts();
      const $resultSet = Rx.Observable.combineLatest(
        Rx.Observable.fromNamedQuerySubscription('draft-list', subscription),
        $pending,
        (resultSet, { drafts: pending, tasks }) => {
          if (!(resultSet instanceof QueryResultSet)) {
            throw 'Set emitResultSet=true and did not receive a QueryResultSet';
          }
          return listResultSet(
            resultSet as QueryResultSet<Message>,
            pending,
            tasks,
            selectionQuery
          );
        }
      );

      this._dataSource = new ObservableListDataSource($resultSet, (range) => {
        retained = range;
        replaceListQuery();
      });
    } else {
      this._dataSource = new ListTabular.DataSource.Empty();
    }

    this.trigger(this);
  };
}

const SEND_LATER = 'send-later';

/**
 * The drafts the list shows first: those with a send queued or running, then those with a
 * send-later expiration, soonest first. A send with no undo delay has no send-later metadata,
 * so its draft comes from the task.
 */
export function pendingSends(withSendLater: Message[], sendTasks: SendDraftTask[]): Message[] {
  const sending = new Set(sendTasks.map((t) => t.headerMessageId));
  const pending = new Map<string, Message>();
  for (const draft of withSendLater) {
    if (sending.has(draft.headerMessageId) || sendLaterExpiration(draft)) {
      pending.set(draft.headerMessageId, draft);
    }
  }
  for (const task of sendTasks) {
    if (task.draft && !pending.has(task.headerMessageId)) {
      pending.set(task.headerMessageId, task.draft);
    }
  }
  const sortKey = (d: Message) => (sending.has(d.headerMessageId) ? 0 : sendLaterExpiration(d));
  return [...pending.values()].sort((a, b) => sortKey(a) - sortKey(b));
}

function sendLaterExpiration(draft: Message): number {
  const metadata = draft.metadataForPluginId(SEND_LATER);
  return metadata && metadata.expiration ? new Date(metadata.expiration).getTime() : 0;
}

/**
 * The list's rows: `pending` at offset 0, then the paged query's (which leaves them out), with
 * the drafts whose send is queued marked by `uploadTaskId`. `query` is the drafts query without
 * the paging or exclusion, so the selection keeps pending drafts.
 */
export function listResultSet(
  resultSet: QueryResultSet<Message>,
  pending: Message[],
  sendTasks: SendDraftTask[],
  query: ModelQuery<Message[]>
) {
  const pendingIds = new Set(pending.map((d) => d.id));
  const rest = resultSet.ids().filter((id) => !pendingIds.has(id));
  const atTop = (resultSet._offset ?? 0) === 0;
  const set = new MutableQueryResultSet<Message>({
    _ids: atTop ? [...pending.map((d) => d.id), ...rest] : rest,
    _offset: atTop ? 0 : (resultSet._offset ?? 0) + pending.length,
  });
  set.setQuery(query);
  rest.forEach((id) => set.updateModel(resultSet.modelWithId(id)));
  if (atTop) pending.forEach((d) => set.updateModel(d));

  const shown = set.models();
  for (const task of sendTasks) {
    const draft = shown.find((d) => d && d.headerMessageId === task.headerMessageId);
    if (draft) {
      const marked = draft.clone() as Message & { uploadTaskId?: string };
      marked.uploadTaskId = task.id;
      set.updateModel(marked);
    }
  }
  return set.immutableClone();
}

export default new DraftListStore();
