import { AccountStore } from '../../src/data/stores/account';
import { MailboxPerspective } from '../../src/data/mailbox-perspective';
import { TaskFactory } from '../../src/data/tasks/factory';
import { Label } from '../../src/data/models/label';
import { Folder } from '../../src/data/models/folder';
import CategoryStore from '../../src/data/stores/category';
import { IntentTask } from '../../src/data/tasks/intent';
import { Thread } from '../../src/data/models/thread';

describe('MailboxPerspective', function mailboxPerspective() {
  beforeEach(() => {
    this.accountIds = ['a1', 'a2'];
    this.accounts = {
      a1: {
        id: 'a1',
        preferredRemovalDestination: () => ({ displayName: 'archive' }),
      },
      a2: {
        id: 'a2',
        preferredRemovalDestination: () => ({ displayName: 'trash2' }),
      },
    };
    this.perspective = new MailboxPerspective(this.accountIds);
    spyOn(AccountStore, 'accountForId').andCallFake((accId) => this.accounts[accId]);
  });

  describe('isEqual', () => {
    // TODO
  });

  describe('canArchiveThreads', () => {
    it('returns false if the perspective is archive', () => {
      const accounts = [{ canArchiveThreads: () => true }, { canArchiveThreads: () => true }];
      spyOn(AccountStore, 'accountsForItems').andReturn(accounts);
      spyOn(this.perspective, 'isArchive').andReturn(true);
      expect(this.perspective.canArchiveThreads()).toBe(false);
    });

    it('returns false if one of the accounts associated with the threads cannot archive', () => {
      const accounts = [{ canArchiveThreads: () => true }, { canArchiveThreads: () => false }];
      spyOn(AccountStore, 'accountsForItems').andReturn(accounts);
      spyOn(this.perspective, 'isArchive').andReturn(false);
      expect(this.perspective.canArchiveThreads()).toBe(false);
    });

    it('returns true otherwise', () => {
      const accounts = [{ canArchiveThreads: () => true }, { canArchiveThreads: () => true }];
      spyOn(AccountStore, 'accountsForItems').andReturn(accounts);
      spyOn(this.perspective, 'isArchive').andReturn(false);
      expect(this.perspective.canArchiveThreads()).toBe(true);
    });
  });

  describe('canMoveThreadsTo', () => {
    it('returns false if the perspective is the target folder', () => {
      const accounts = [{ id: 'a' }, { id: 'b' }];
      spyOn(AccountStore, 'accountsForItems').andReturn(accounts);
      spyOn(this.perspective, 'categoriesSharedRole').andReturn('trash');
      expect(this.perspective.canMoveThreadsTo([], 'trash')).toBe(false);
    });

    it('returns false if one of the accounts associated with the threads does not have the folder', () => {
      const accounts = [{ id: 'a' }, { id: 'b' }];
      spyOn(CategoryStore, 'getCategoryByRole').andReturn(null);
      spyOn(AccountStore, 'accountsForItems').andReturn(accounts);
      spyOn(this.perspective, 'categoriesSharedRole').andReturn('inbox');
      expect(this.perspective.canMoveThreadsTo([], 'trash')).toBe(false);
    });

    it('returns true otherwise', () => {
      const accounts = [{ id: 'a' }, { id: 'b' }];
      const category = { id: 'cat' };
      spyOn(CategoryStore, 'getCategoryByRole').andReturn(category);
      spyOn(AccountStore, 'accountsForItems').andReturn(accounts);
      spyOn(this.perspective, 'categoriesSharedRole').andReturn('inbox');
      expect(this.perspective.canMoveThreadsTo([], 'trash')).toBe(true);
    });
  });

  describe('canReceiveThreadsFromAccountIds', () => {
    it('returns true if the thread account ids are included in the current account ids', () => {
      expect(this.perspective.canReceiveThreadsFromAccountIds(['a1'])).toBe(true);
    });

    it('returns false otherwise', () => {
      expect(this.perspective.canReceiveThreadsFromAccountIds(['a4'])).toBe(false);
      expect(this.perspective.canReceiveThreadsFromAccountIds([])).toBe(false);
      expect(this.perspective.canReceiveThreadsFromAccountIds()).toBe(false);
    });
  });

  describe('CategoryMailboxPerspective', () => {
    beforeEach(() => {
      this.categories = [
        new Label({ path: 'c1', accountId: 'a1' }),
        new Label({ path: 'c2', accountId: 'a2' }),
        new Label({ path: 'c3', accountId: 'a2' }),
      ];
      this.perspective = MailboxPerspective.forCategories(this.categories);
    });

    describe('canReceiveThreadsFromAccountIds', () => {
      it('returns true if the thread account ids are included in the current account ids', () => {
        expect(this.perspective.canReceiveThreadsFromAccountIds(['a1'])).toBe(true);
      });

      it('returns false otherwise', () => {
        expect(this.perspective.canReceiveThreadsFromAccountIds(['a4'])).toBe(false);
        expect(this.perspective.canReceiveThreadsFromAccountIds([])).toBe(false);
        expect(this.perspective.canReceiveThreadsFromAccountIds()).toBe(false);
      });

      it('returns false if it is a locked category', () => {
        this.perspective._categories.push(new Label({ role: 'sent', path: 'c4', accountId: 'a1' }));
        expect(this.perspective.canReceiveThreadsFromAccountIds(['a2'])).toBe(false);
      });
    });

    describe('receiveThreads', () => {
      // TODO
    });
  });

  // What removing means on each account is the engine's (threads.removeFromView); the
  // perspective names the view and words the swipe and the toast.
  describe('tasksForRemovingItems', () => {
    const inbox = new Folder({ id: 'inbox-1', accountId: 'a1', role: 'inbox', path: 'INBOX' });
    const thread = new Thread({ id: 't1', accountId: 'a1' });

    it('asks the engine to remove them from the view', () => {
      this.accounts.a1.preferredRemovalDestination = () => ({ role: 'archive' });
      const perspective = MailboxPerspective.forCategories([inbox]);
      const [task] = perspective.tasksForRemovingItems([thread], 'Swipe') as IntentTask[];
      expect(task instanceof IntentTask).toBe(true);
      expect(task.intent).toBe('threads.removeFromView');
      expect(task.params).toEqual({ threadIds: ['t1'], perspective: { categoryIds: ['inbox-1'] } });
      expect(task.description()).toBe('Archived');
      expect(perspective.removalKind([thread])).toBe('archive');
    });

    it('passes the Delete preference and the unread view', () => {
      AppEnv.config.set('core.reading.backspaceDelete', true);
      this.accounts.a1.preferredRemovalDestination = () => ({ role: 'trash' });
      const perspective = MailboxPerspective.forUnread([inbox]);
      const [task] = perspective.tasksForRemovingItems([thread]) as IntentTask[];
      expect(task.params).toEqual({
        threadIds: ['t1'],
        perspective: { categoryIds: ['inbox-1'], unread: true },
        preferTrash: true,
      });
      expect(perspective.removalKind([thread])).toBe('trash');
      AppEnv.config.set('core.reading.backspaceDelete', false);
    });

    it('removes nothing from Spam or Trash, and unstars in Starred', () => {
      const spam = new Folder({ id: 'spam-1', accountId: 'a1', role: 'spam', path: 'Spam' });
      expect(MailboxPerspective.forCategories([spam]).tasksForRemovingItems([thread])).toEqual([]);
      const starred = MailboxPerspective.forStarred(['a1']);
      const [task] = starred.tasksForRemovingItems([thread]) as IntentTask[];
      expect(task.params).toEqual({ threadIds: ['t1'], perspective: { starred: true } });
      expect(starred.removalKind([thread])).toBe('unstar');
    });
  });

  describe('sourceFolderIdsForAccount', () => {
    it('returns the folder the perspective shows for each account of a unified view', () => {
      const perspective = MailboxPerspective.forCategories([
        new Folder({ id: 'inbox-1', accountId: 'a1', role: 'inbox', path: 'INBOX' } as any),
        new Folder({ id: 'inbox-2', accountId: 'a2', role: 'inbox', path: 'INBOX' } as any),
      ]);
      expect(perspective.sourceFolderIdsForAccount('a1')).toEqual(['inbox-1']);
      expect(perspective.sourceFolderIdsForAccount('a2')).toEqual(['inbox-2']);
      expect(perspective.sourceFolderIdsForAccount('a3')).toEqual([]);
    });

    it('returns nothing for a label, which holds no copy of its own', () => {
      const perspective = MailboxPerspective.forCategories([
        new Label({ id: 'label-1', accountId: 'a1', path: 'Receipts' } as any),
      ]);
      expect(perspective.sourceFolderIdsForAccount('a1')).toEqual([]);
    });

    it('returns nothing for a perspective without categories', () => {
      expect(MailboxPerspective.forStarred(['a1']).sourceFolderIdsForAccount('a1')).toEqual([]);
    });
  });
});
