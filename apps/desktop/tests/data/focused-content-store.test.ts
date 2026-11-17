import { Thread } from '../../src/data/models/thread';
import FocusedContentStore from '../../src/data/stores/focused-content';

const testThread = new Thread({ id: '123', accountId: TEST_ACCOUNT_ID });

/**
 * The store is the window's singleton and MessageStore follows it, so a focus or an animation
 * frame trigger left behind opens thread 123 on a later spec's engine. Every spec stubs the
 * trigger and clears the focus when it ends.
 */
describe('FocusedContentStore', function () {
  beforeEach(function () {
    spyOn(FocusedContentStore, 'triggerAfterAnimationFrame');
  });

  afterEach(function () {
    FocusedContentStore._resetInstanceVars();
  });

  /** Focuses `item` and forgets the trigger that caused. */
  const focus = (item: Thread | null) => {
    FocusedContentStore._onFocus({ collection: 'thread', item });
    (FocusedContentStore.triggerAfterAnimationFrame as jasmine.Spy).reset();
  };

  describe('onSetFocus', function () {
    it('should not trigger if the thread is already focused', function () {
      focus(testThread);
      FocusedContentStore._onFocus({ collection: 'thread', item: testThread });
      expect(FocusedContentStore.triggerAfterAnimationFrame).not.toHaveBeenCalled();
    });

    it('should not trigger if the focus is already null', function () {
      focus(null);
      FocusedContentStore._onFocus({ collection: 'thread', item: null });
      expect(FocusedContentStore.triggerAfterAnimationFrame).not.toHaveBeenCalled();
    });

    it('should trigger otherwise', function () {
      focus(null);
      FocusedContentStore._onFocus({ collection: 'thread', item: testThread });
      expect(FocusedContentStore.triggerAfterAnimationFrame).toHaveBeenCalled();
    });
  });

  describe('threadId', () =>
    it('should return the id of the focused thread', function () {
      focus(testThread);
      expect(FocusedContentStore.focusedId('thread')).toBe(testThread.id);
    }));

  describe('thread', () =>
    it('should return the focused thread object', function () {
      focus(testThread);
      expect(FocusedContentStore.focused('thread')).toBe(testThread);
    }));
});
