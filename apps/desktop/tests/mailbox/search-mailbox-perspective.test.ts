import { AccountStore } from '../../src/data/stores/account';
import { IntentTask } from '../../src/data/tasks/intent';
import { Folder } from '../../src/data/models/folder';
import { Thread } from '../../src/data/models/thread';
import SearchMailboxPerspective from '../../src/features/thread-search/mailbox-perspective';

// Search results span every folder: "remove" is each account's archive or trash intent with
// no view, so the engine moves every copy outside Sent and Drafts (on Gmail, archiving removes
// the Inbox label; tern-tasks intents.rs).
describe('SearchMailboxPerspective', function searchMailboxPerspective() {
  beforeEach(() => {
    this.sourcePerspective = { accountIds: ['a1', 'a2'], name: 'Inbox' };
    this.perspective = new SearchMailboxPerspective(this.sourcePerspective, 'from:ben');
    this.threads = [
      new Thread({ id: 't1', accountId: 'a1' }),
      new Thread({ id: 't2', accountId: 'a2' }),
    ];
    this.destinations = {
      a1: new Folder({ id: 'archive', accountId: 'a1', role: 'archive' }),
      a2: new Folder({ id: 'trash', accountId: 'a2', role: 'trash' }),
    };
    spyOn(AccountStore, 'accountForId').andCallFake((id) => ({
      id,
      preferredRemovalDestination: () => this.destinations[id],
    }));
  });

  describe('tasksForRemovingItems', () => {
    it("takes each account's preferred removal, unscoped", () => {
      const tasks: IntentTask[] = this.perspective.tasksForRemovingItems(this.threads);

      expect(tasks.map((t) => [t.intent, t.params])).toEqual([
        ['threads.trash', { threadIds: ['t2'] }],
        ['threads.archive', { threadIds: ['t1'] }],
      ]);
      expect(tasks.every((t) => t instanceof IntentTask)).toBe(true);
      expect(() => tasks.forEach((t) => t.willBeQueued())).not.toThrow();
    });

    it('archives Gmail through the archive intent, whose destination is All Mail', () => {
      this.destinations.a1 = new Folder({ id: 'all', accountId: 'a1', role: 'all' });
      const [task] = this.perspective.tasksForRemovingItems([this.threads[0]]);
      expect(task.intent).toBe('threads.archive');
      expect(this.perspective.removalKind([this.threads[0]])).toBe('archive');
    });

    it('does nothing for an account with nowhere to put them', () => {
      this.destinations.a1 = null;
      expect(this.perspective.tasksForRemovingItems([this.threads[0]])).toEqual([]);
      expect(this.perspective.removalKind([this.threads[0]])).toBe(null);
    });
  });
});
