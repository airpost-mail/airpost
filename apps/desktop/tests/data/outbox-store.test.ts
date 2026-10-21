import OutboxStore from '../../src/data/stores/outbox';
import TaskQueue from '../../src/data/stores/task-queue';
import { SendDraftTask } from '../../src/data/tasks/send-draft';
import { SyncbackDraftTask } from '../../src/data/tasks/syncback-draft';
import { MailboxPerspective } from '../../src/data/mailbox-perspective';
import { Message } from '../../src/data/models/message';

describe('OutboxStore', function () {
  afterEach(function () {
    (OutboxStore as any)._tasks = [];
  });

  it("matches send and syncback tasks on their draft's account", function () {
    const send = new SendDraftTask({});
    send.draft = new Message({ accountId: 'a1', headerMessageId: 'h1' });
    const syncback = new SyncbackDraftTask({
      draft: new Message({ accountId: 'a2', headerMessageId: 'h2' }),
    });
    spyOn(TaskQueue, 'queue').andReturn([send, syncback]);
    (OutboxStore as any)._populate();

    expect(OutboxStore.itemsForAccount('a1')).toEqual([send]);
    expect(OutboxStore.itemsForAccount('a2')).toEqual([syncback]);
    expect(MailboxPerspective.forDrafts(['a1', 'a2']).unreadCount()).toBe(2);
  });
});
