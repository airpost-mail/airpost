import * as Actions from '../../src/data/actions';
import { Message } from '../../src/data/models/message';
import DatabaseStore from '../../src/data/database/store';
import FocusedContentStore from '../../src/data/stores/focused-content';
import DraftListStore from '../../src/features/draft-list/store';

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
});
