import * as Actions from '../../src/data/actions';
import { Message } from '../../src/data/models/message';
import DatabaseStore from '../../src/data/database/store';
import FocusedContentStore from '../../src/data/stores/focused-content';
import FocusedPerspectiveStore from '../../src/data/stores/focused-perspective';
import OutboxStore from '../../src/data/stores/outbox';
import DraftListStore, { listResultSet, pendingSends } from '../../src/features/draft-list/store';
import { SendDraftTask } from '../../src/data/tasks/send-draft';
import { MutableQueryResultSet } from '../../src/data/query/mutable-result-set';
import { QueryRange } from '../../src/data/query/range';

describe('DraftListStore', function () {
  const draft = new Message({
    id: 'draft-1',
    headerMessageId: 'hmid-1',
    accountId: TEST_ACCOUNT_ID,
    draft: true,
  });

  beforeEach(() => {
    DraftListStore._focusedHeaderMessageId = null;
    spyOn(Actions, 'setFocus');
  });

  describe('_onSetFocus', () => {
    it('tracks the headerMessageId of the focused draft', () => {
      DraftListStore._onSetFocus({ collection: 'draft', item: draft });
      expect(DraftListStore.focusedHeaderMessageId()).toBe('hmid-1');
      DraftListStore._onSetFocus({ collection: 'draft', item: null });
      expect(DraftListStore.focusedHeaderMessageId()).toBe(null);
    });

    it('ignores other collections', () => {
      DraftListStore._onSetFocus({ collection: 'thread', item: draft });
      expect(DraftListStore.focusedHeaderMessageId()).toBe(null);
    });
  });

  describe('_onDataChanged', () => {
    beforeEach(() => {
      DraftListStore._focusedHeaderMessageId = 'hmid-1';
      spyOn(DraftListStore, '_resolveFocusedDraft');
    });

    it('does not look the draft up again when the focused draft is saved', () => {
      spyOn(FocusedContentStore, 'focusedId').andReturn('draft-1');
      DraftListStore._onDataChanged({ type: 'persist', objectClass: 'Message', objects: [draft] });
      expect(DraftListStore._resolveFocusedDraft).not.toHaveBeenCalled();
    });

    it('looks the draft up when it reappears under a new id', () => {
      spyOn(FocusedContentStore, 'focusedId').andReturn('draft-1');
      const moved = new Message({ id: 'draft-2', headerMessageId: 'hmid-1', draft: true });
      DraftListStore._onDataChanged({ type: 'persist', objectClass: 'Message', objects: [moved] });
      expect(DraftListStore._resolveFocusedDraft).toHaveBeenCalledWith('hmid-1');
    });

    it('looks the draft up when it is unpersisted', () => {
      spyOn(FocusedContentStore, 'focusedId').andReturn('draft-1');
      DraftListStore._onDataChanged({
        type: 'unpersist',
        objectClass: 'Message',
        objects: [draft],
      });
      expect(DraftListStore._resolveFocusedDraft).toHaveBeenCalledWith('hmid-1');
    });

    it('ignores other drafts', () => {
      const other = new Message({ id: 'draft-9', headerMessageId: 'hmid-9', draft: true });
      DraftListStore._onDataChanged({
        type: 'unpersist',
        objectClass: 'Message',
        objects: [other],
      });
      expect(DraftListStore._resolveFocusedDraft).not.toHaveBeenCalled();
    });
  });

  describe('_resolveFocusedDraft', () => {
    beforeEach(() => {
      DraftListStore._focusedHeaderMessageId = 'hmid-1';
    });

    it('focuses the draft under its new id', () => {
      const moved = new Message({ id: 'draft-2', headerMessageId: 'hmid-1', draft: true });
      spyOn(DatabaseStore, 'run').andReturn(Promise.resolve(moved));
      spyOn(FocusedContentStore, 'focusedId').andReturn(null);
      waitsForPromise(async () => {
        await DraftListStore._resolveFocusedDraft('hmid-1');
        expect(Actions.setFocus).toHaveBeenCalledWith({ collection: 'draft', item: moved });
      });
    });

    it('clears focus when the draft is gone', () => {
      spyOn(DatabaseStore, 'run').andReturn(Promise.resolve(null));
      waitsForPromise(async () => {
        await DraftListStore._resolveFocusedDraft('hmid-1');
        expect(Actions.setFocus).toHaveBeenCalledWith({ collection: 'draft', item: null });
      });
    });

    it('does nothing if focus moved on during the lookup', () => {
      spyOn(DatabaseStore, 'run').andCallFake(() => {
        DraftListStore._focusedHeaderMessageId = 'hmid-other';
        return Promise.resolve(null);
      });
      waitsForPromise(async () => {
        await DraftListStore._resolveFocusedDraft('hmid-1');
        expect(Actions.setFocus).not.toHaveBeenCalled();
      });
    });
  });

  describe('_onPopoutDraft', () => {
    it('releases the previewed draft when it pops out', () => {
      DraftListStore._focusedHeaderMessageId = 'hmid-1';
      DraftListStore._onPopoutDraft('hmid-1');
      expect(Actions.setFocus).toHaveBeenCalledWith({ collection: 'draft', item: null });
    });

    it('keeps the preview when another draft pops out', () => {
      DraftListStore._focusedHeaderMessageId = 'hmid-1';
      DraftListStore._onPopoutDraft('hmid-2');
      expect(Actions.setFocus).not.toHaveBeenCalled();
    });
  });

  describe('draftsForCommand', () => {
    const cursorDraft = new Message({ id: 'draft-3', headerMessageId: 'hmid-3', draft: true });

    let realDataSource;
    let selected: Message[];

    beforeEach(() => {
      selected = [];
      realDataSource = DraftListStore._dataSource;
      DraftListStore._dataSource = { selection: { items: () => selected } } as any;
      spyOn(FocusedContentStore, 'keyboardCursorEnabled').andReturn(true);
      spyOn(FocusedContentStore, 'keyboardCursor').andReturn(cursorDraft);
    });

    afterEach(() => {
      DraftListStore._dataSource = realDataSource;
    });

    it('prefers the selection', () => {
      selected = [cursorDraft, draft];
      spyOn(FocusedContentStore, 'focused').andReturn(draft);
      expect(DraftListStore.draftsForCommand()).toEqual([cursorDraft, draft]);
    });

    it('falls back to the focused draft', () => {
      spyOn(FocusedContentStore, 'focused').andReturn(draft);
      expect(DraftListStore.draftsForCommand()).toEqual([draft]);
    });

    it('falls back to the keyboard cursor', () => {
      spyOn(FocusedContentStore, 'focused').andReturn(null);
      expect(DraftListStore.draftsForCommand()).toEqual([cursorDraft]);
    });
  });

  describe('pending sends', () => {
    const at = (ms: number) => new Date(1_800_000_000_000 + ms);
    const make = (n: number, sendLater?: Record<string, unknown>) => {
      const m = new Message({
        id: `d${n}`,
        headerMessageId: `h${n}`,
        accountId: TEST_ACCOUNT_ID,
        draft: true,
      });
      return (sendLater ? m.directlyAttachMetadata('send-later', sendLater) : m) as Message;
    };
    const taskFor = (draft: Message) => {
      const task = new SendDraftTask({});
      task.id = `task-${draft.id}`;
      task.headerMessageId = draft.headerMessageId;
      task.draft = draft;
      return task;
    };

    it('puts sending drafts first, then scheduled ones soonest first', () => {
      const later = make(1, { expiration: at(3600 * 1000) });
      const soon = make(2, { expiration: at(5000), isUndoSend: true });
      const cancelled = make(3, { expiration: null, isUndoSend: true });
      const sending = make(4, { expiration: null });
      const noDelay = make(5);
      const tasks = [taskFor(sending), taskFor(noDelay)];
      const ids = pendingSends([later, soon, cancelled, sending], tasks).map((d) => d.id);
      expect(ids.slice(0, 2).sort()).toEqual(['d4', 'd5']);
      expect(ids.slice(2)).toEqual(['d2', 'd1']);
    });

    const page = (models: Message[], offset: number) => {
      const set = new MutableQueryResultSet<Message>();
      set.addModelsInRange(models, new QueryRange({ offset, limit: models.length }));
      return set;
    };
    const query = DatabaseStore.findAll<Message>(Message).where({ draft: true });

    it('lists the pending drafts above the paged ones and marks queued sends', () => {
      const scheduled = make(1, { expiration: at(5000) });
      const sending = make(2, { expiration: null });
      const rest = [make(3), make(4)];
      // The paged query may still hold a pending draft until it reruns without it.
      const set = listResultSet(
        page([rest[0], sending, rest[1]], 0),
        [sending, scheduled],
        [taskFor(sending)],
        query
      );
      expect(set.ids()).toEqual(['d2', 'd1', 'd3', 'd4']);
      expect(set.range().offset).toBe(0);
      expect((set.modelWithId('d2') as any).uploadTaskId).toBe('task-d2');
      expect((set.modelWithId('d3') as any).uploadTaskId).toBe(undefined);
      expect(set.query().matchers()).toEqual(query.matchers());
    });

    it('shifts a later page by the number of pending drafts', () => {
      const set = listResultSet(
        page([make(3), make(4)], 100),
        [make(1, { expiration: at(5000) })],
        [],
        query
      );
      expect(set.ids()).toEqual(['d3', 'd4']);
      expect(set.offsetOfId('d3')).toBe(101);
    });

    describe('in the list', () => {
      let drafts: Message[];

      beforeEach(() => {
        spyOn(FocusedPerspectiveStore, 'current').andReturn({
          drafts: true,
          accountIds: [TEST_ACCOUNT_ID],
        } as any);
        // Answers each query from `drafts` (newest first) with its own matchers and range.
        spyOn(DatabaseStore, 'run').andCallFake((q: any) => {
          let rows = drafts.filter((m) => q.matchers().every((matcher) => matcher.evaluate(m)));
          const range = q.range();
          if (!range.isInfinite()) rows = rows.slice(range.offset, range.offset + range.limit);
          return Promise.resolve(q._returnIds ? rows.map((m) => m.id) : rows);
        });
      });

      afterEach(() => {
        DraftListStore._dataSource.cleanup();
        (OutboxStore as any)._tasks = [];
      });

      const rows = async () => {
        for (let i = 0; i < 20; i++) await Promise.resolve();
        const source = DraftListStore.dataSource() as any;
        return source._resultSet.models().map((m: Message) => m.id);
      };

      it('lists scheduled and sending drafts first and the rest once', async () => {
        const noDelay = make(4);
        drafts = [make(1), make(2, { expiration: at(5000) }), make(3), noDelay];
        (OutboxStore as any)._tasks = [taskFor(noDelay)];
        DraftListStore._createListDataSource();
        DraftListStore.dataSource().setRetainedRange({ start: 0, end: 10 });
        expect(await rows()).toEqual(['d4', 'd2', 'd1', 'd3']);
      });

      it('puts a draft back in date order once its send is cancelled', async () => {
        drafts = [make(1), make(2, { expiration: at(5000) }), make(3)];
        DraftListStore._createListDataSource();
        DraftListStore.dataSource().setRetainedRange({ start: 0, end: 10 });
        expect(await rows()).toEqual(['d2', 'd1', 'd3']);

        drafts[1] = make(2, { expiration: null, isUndoSend: true });
        DatabaseStore.trigger({ type: 'persist', objectClass: 'Message', objects: [drafts[1]] });
        expect(await rows()).toEqual(['d1', 'd2', 'd3']);
      });
    });
  });
});
