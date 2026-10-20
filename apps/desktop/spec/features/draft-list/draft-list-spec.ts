import { Actions, Message, WorkspaceStore, FocusedContentStore } from 'mailspring-exports';
import DraftList from '../../../src/features/draft-list/draft-list';
import DraftListStore from '../../../src/features/draft-list/store';

describe('DraftList', function () {
  const cursor = new Message({ id: 'draft-1', headerMessageId: 'hmid-1', draft: true });
  let list: any;

  beforeEach(() => {
    list = new DraftList({});
    spyOn(Actions, 'setFocus');
    spyOn(Actions, 'destroyDraft');
    spyOn(FocusedContentStore, 'keyboardCursor').andReturn(cursor);
  });

  describe('Enter', () => {
    it('moves into the preview composer in split mode', () => {
      spyOn(WorkspaceStore, 'layoutMode').andReturn('split');
      spyOn(DraftListStore, 'focusPreview').andReturn(true);
      list._keymapHandlers()['core:focus-item']();
      expect(DraftListStore.focusPreview).toHaveBeenCalled();
      expect(Actions.setFocus).not.toHaveBeenCalled();
    });

    it('focuses the cursor draft in split mode when no preview is showing', () => {
      spyOn(WorkspaceStore, 'layoutMode').andReturn('split');
      spyOn(DraftListStore, 'focusPreview').andReturn(false);
      list._keymapHandlers()['core:focus-item']();
      expect(Actions.setFocus).toHaveBeenCalledWith({ collection: 'draft', item: cursor });
    });

    it('opens the cursor draft in list mode', () => {
      spyOn(WorkspaceStore, 'layoutMode').andReturn('list');
      spyOn(DraftListStore, 'focusPreview');
      list._keymapHandlers()['core:focus-item']();
      expect(DraftListStore.focusPreview).not.toHaveBeenCalled();
      expect(Actions.setFocus).toHaveBeenCalledWith({ collection: 'draft', item: cursor });
    });
  });

  it('deletes the drafts the store resolves for the command', () => {
    spyOn(DraftListStore, 'draftsForCommand').andReturn([cursor]);
    list._keymapHandlers()['core:delete-item']();
    expect(Actions.destroyDraft).toHaveBeenCalledWith(cursor);
  });
});
