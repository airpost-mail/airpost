import * as Actions from '../../src/data/actions';
import WorkspaceStore from '../../src/data/stores/workspace';
import { Message } from '../../src/data/models/message';

describe('WorkspaceStore draft focus', function () {
  const draft = new Message({ id: 'draft-1', headerMessageId: 'hmid-1', draft: true });
  const store = WorkspaceStore as any;
  let definedHere: string[];
  let savedStack;
  let savedMode;

  beforeEach(() => {
    definedHere = [];
    if (!WorkspaceStore.Sheet.Drafts) {
      WorkspaceStore.defineSheet('Drafts', { root: true, silent: true } as any, {
        list: ['RootSidebar', 'DraftList'],
        split: ['RootSidebar', 'DraftList', 'DraftPreview'],
      });
      definedHere.push('Drafts');
    }
    if (!WorkspaceStore.Sheet.Draft) {
      WorkspaceStore.defineSheet('Draft', {}, { list: ['DraftPreview'] });
      definedHere.push('Draft');
    }
    savedStack = store._sheetStack;
    savedMode = store._preferredLayoutMode;
    store._sheetStack = [WorkspaceStore.Sheet.Drafts];
    spyOn(WorkspaceStore, 'trigger');
  });

  afterEach(() => {
    store._sheetStack = savedStack;
    store._preferredLayoutMode = savedMode;
    definedHere.forEach((id) => WorkspaceStore.undefineSheet(id));
  });

  describe('in list mode', () => {
    beforeEach(() => {
      store._preferredLayoutMode = 'list';
    });

    it('pushes the Draft sheet when a draft is focused', () => {
      store._onSetFocus({ collection: 'draft', item: draft });
      expect(WorkspaceStore.topSheet()).toBe(WorkspaceStore.Sheet.Draft);
    });

    it('does not push the Draft sheet twice', () => {
      store._onSetFocus({ collection: 'draft', item: draft });
      store._onSetFocus({ collection: 'draft', item: draft });
      expect(WorkspaceStore.sheetStack().length).toBe(2);
    });

    it('pops the Draft sheet when focus is cleared', () => {
      store._onSetFocus({ collection: 'draft', item: draft });
      store._onSetFocus({ collection: 'draft', item: null });
      expect(WorkspaceStore.topSheet()).toBe(WorkspaceStore.Sheet.Drafts);
    });

    it('clears draft focus when the Draft sheet is popped', () => {
      store._onSetFocus({ collection: 'draft', item: draft });
      spyOn(Actions, 'setFocus');
      WorkspaceStore.popSheet();
      expect(Actions.setFocus).toHaveBeenCalledWith({ collection: 'draft', item: null });
    });
  });

  describe('in split mode', () => {
    it('keeps the Drafts sheet on top', () => {
      store._preferredLayoutMode = 'split';
      store._onSetFocus({ collection: 'draft', item: draft });
      expect(WorkspaceStore.topSheet()).toBe(WorkspaceStore.Sheet.Drafts);
    });
  });
});
