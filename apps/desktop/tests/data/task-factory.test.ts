import { TaskFactory } from '../../src/data/tasks/factory';
import { IntentTask } from '../../src/data/tasks/intent';
import { ChangeUnreadTask } from '../../src/data/tasks/change-unread';
import { Folder } from '../../src/data/models/folder';
import { Thread } from '../../src/data/models/thread';

// What each action means on each account (Gmail labels, the copies a move takes) is the
// engine's (tern-tasks intents.rs); the factory names the intent and the view.
describe('TaskFactory', function taskFactory() {
  beforeEach(() => {
    this.threads = [
      new Thread({ id: 't1', accountId: 'ac-1', unread: true, starred: false }),
      new Thread({ id: 't2', accountId: 'ac-2', unread: false, starred: false }),
    ];
  });

  const only = (tasks: any[]) => {
    expect(tasks.length).toBe(1);
    expect(tasks[0] instanceof IntentTask).toBe(true);
    return tasks[0] as IntentTask;
  };

  it('archives threads of every account in one intent, scoped to the view', () => {
    const perspective = {
      categories: () => [new Folder({ id: 'f-sent', accountId: 'ac-1', role: 'sent' } as any)],
    };
    const task = only(
      TaskFactory.tasksForArchiving({ threads: this.threads, source: 'Toolbar', perspective })
    );
    expect(task.intent).toBe('threads.archive');
    expect(task.params).toEqual({
      threadIds: ['t1', 't2'],
      perspective: { categoryIds: ['f-sent'] },
    });
    expect(task.threadIds).toEqual(['t1', 't2']);
    expect(task.accountId).toBe('ac-1');
    expect(task.source).toBe('Toolbar');
  });

  it('leaves the copies to the engine without a view, or with a v1 one', () => {
    const v1 = { sourceFolderIdsForAccount: () => ['f-sent'] } as any;
    for (const perspective of [undefined, v1]) {
      const task = only(
        TaskFactory.tasksForArchiving({ threads: this.threads, source: 'MCP', perspective })
      );
      expect(task.params).toEqual({ threadIds: ['t1', 't2'] });
    }
  });

  it('names the spam, not-spam and trash intents', () => {
    const args = { threads: this.threads, source: 'Toolbar' };
    expect(only(TaskFactory.tasksForMarkingAsSpam(args)).intent).toBe('threads.spam');
    expect(only(TaskFactory.tasksForMarkingNotSpam(args)).intent).toBe('threads.notSpam');
    expect(only(TaskFactory.tasksForMovingToTrash(args)).intent).toBe('threads.trash');
    expect(TaskFactory.tasksForMovingToTrash({ threads: [], source: 'x' })).toEqual([]);
  });

  it('inverts unread and starred', () => {
    const unread = TaskFactory.taskForInvertingUnread({ threads: this.threads, source: 'x' });
    expect((unread as IntentTask).intent).toBe('threads.markRead');
    expect((unread as IntentTask).params).toEqual({ threadIds: ['t1', 't2'] });
    const read = [this.threads[1]];
    const markUnread = TaskFactory.taskForInvertingUnread({ threads: read, source: 'x' });
    expect((markUnread as IntentTask).params).toEqual({ threadIds: ['t2'], unread: true });
    const star = TaskFactory.taskForInvertingStarred({ threads: this.threads, source: 'x' });
    expect((star as IntentTask).intent).toBe('threads.star');
    expect((star as IntentTask).params).toEqual({ threadIds: ['t1', 't2'], starred: true });
  });

  it('marks read without an undo entry as a plain task', () => {
    const task = TaskFactory.taskForSettingUnread({
      threads: [this.threads[0]],
      unread: false,
      source: 'Thread Selected',
      canBeUndone: false,
    });
    expect(task instanceof ChangeUnreadTask).toBe(true);
    expect(task.canBeUndone).toBe(false);
  });
});
