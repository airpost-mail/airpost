import { Folder } from '../../src/data/models/folder';
import * as Actions from '../../src/data/actions';
import { IntentTask } from '../../src/data/tasks/intent';
import SidebarItem from '../../src/features/account-sidebar/sidebar-item';

describe('sidebar-item', function sidebarItemSpec() {
  it('preserves nested labels on rename', () => {
    const queueTask = spyOn(Actions, 'queueTask');
    const categories = [new Folder({ path: 'a.b/c', accountId: TEST_ACCOUNT_ID })];
    AppEnv.savedState.sidebarKeysCollapsed = {};
    const item = SidebarItem.forCategories(categories) as any;
    item.onEdited(item, 'd');

    const task = queueTask.calls[0].args[0];
    expect(task.intent).toBe('categories.rename');
    expect(task.params).toEqual({ categoryId: categories[0].id, name: 'a.b/d' });
  });
  it('preserves labels on rename', () => {
    const queueTask = spyOn(Actions, 'queueTask');
    const categories = [new Folder({ path: 'a', accountId: TEST_ACCOUNT_ID })];
    AppEnv.savedState.sidebarKeysCollapsed = {};
    const item = SidebarItem.forCategories(categories);
    item.onEdited(item, 'b') as any;

    const task = queueTask.calls[0].args[0];
    expect(task.params).toEqual({ categoryId: categories[0].id, name: 'b' });
  });

  describe('onMarkAllAsRead', () => {
    beforeEach(() => {
      AppEnv.savedState.sidebarKeysCollapsed = {};
    });

    // The engine finds the category's unread threads (categories.markRead).
    it('queues the mark-read intent for the category', () => {
      const queueTask = spyOn(Actions, 'queueTask');
      const categories = [new Folder({ id: 'f-a', path: 'a', accountId: TEST_ACCOUNT_ID })];
      const item = SidebarItem.forCategories(categories) as any;

      item.onMarkAllAsRead(item);

      const task = queueTask.calls[0].args[0];
      expect(task instanceof IntentTask).toBe(true);
      expect(task.intent).toBe('categories.markRead');
      expect(task.params).toEqual({ categoryId: 'f-a' });
      expect(task.accountId).toBe(TEST_ACCOUNT_ID);
    });

    it('is present on items built via forCategories', () => {
      const categories = [new Folder({ path: 'a', accountId: TEST_ACCOUNT_ID })];
      const item = SidebarItem.forCategories(categories) as any;
      expect(item.onMarkAllAsRead).toBeDefined();
    });

    it('is not present on items built via forUnread', () => {
      const item = SidebarItem.forUnread([TEST_ACCOUNT_ID]) as any;
      expect(item.onMarkAllAsRead).toBeUndefined();
    });

    it('is not present on items built via forStarred', () => {
      const item = SidebarItem.forStarred([TEST_ACCOUNT_ID]) as any;
      expect(item.onMarkAllAsRead).toBeUndefined();
    });

    it('is not present on items built via forDrafts', () => {
      const item = SidebarItem.forDrafts([TEST_ACCOUNT_ID], { name: 'test' }) as any;
      expect(item.onMarkAllAsRead).toBeUndefined();
    });
  });
});
