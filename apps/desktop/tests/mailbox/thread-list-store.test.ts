import * as Actions from '../../src/data/actions';
import { Folder } from '../../src/data/models/folder';
import { MailboxPerspective } from '../../src/data/mailbox-perspective';
import FocusedPerspectiveStore from '../../src/data/stores/focused-perspective';
import ThreadListStore from '../../src/features/thread-list/store';

describe('ThreadListStore', function () {
  beforeEach(function () {
    this.current = MailboxPerspective.forCategory(
      new Folder({ id: 'id-456', path: 'Projects', accountId: TEST_ACCOUNT_ID })
    );
    spyOn(FocusedPerspectiveStore, 'current').andCallFake(() => this.current);
    ThreadListStore.createListDataSource();
    this.dataSource = ThreadListStore.dataSource();
    spyOn(Actions, 'setFocus');
  });

  it('keeps the data source and the open thread when the focused folder is renamed', function () {
    // FocusedPerspectiveStore._refreshRenamedCategories: same id, new path.
    this.current = MailboxPerspective.forCategory(
      new Folder({ id: 'id-456', path: 'Clients', accountId: TEST_ACCOUNT_ID })
    );
    spyOn(ThreadListStore, 'trigger');
    ThreadListStore._onPerspectiveChanged();
    expect(ThreadListStore.dataSource()).toBe(this.dataSource);
    expect(Actions.setFocus).not.toHaveBeenCalled();
    expect(ThreadListStore.trigger).toHaveBeenCalled();
  });

  it('rebuilds for another folder', function () {
    this.current = MailboxPerspective.forCategory(
      new Folder({ id: 'id-789', path: 'Clients', accountId: TEST_ACCOUNT_ID })
    );
    ThreadListStore._onPerspectiveChanged();
    expect(ThreadListStore.dataSource()).not.toBe(this.dataSource);
    expect(Actions.setFocus).toHaveBeenCalledWith({ collection: 'thread', item: null });
  });

  it('rebuilds when the folder gains a role its query depends on', function () {
    this.current = MailboxPerspective.forCategory(
      new Folder({ id: 'id-456', path: 'Projects', role: 'trash', accountId: TEST_ACCOUNT_ID })
    );
    ThreadListStore._onPerspectiveChanged();
    expect(ThreadListStore.dataSource()).not.toBe(this.dataSource);
  });

  it('always rebuilds perspectives without categories', function () {
    this.current = MailboxPerspective.forStarred([TEST_ACCOUNT_ID]);
    ThreadListStore.createListDataSource();
    const before = ThreadListStore.dataSource();
    ThreadListStore._onPerspectiveChanged();
    expect(ThreadListStore.dataSource()).not.toBe(before);
  });
});
