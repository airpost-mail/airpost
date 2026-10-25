import { Actions, FocusedContentStore, Thread, SyncbackMetadataTask } from 'mailspring-exports';

import SendRemindersStore from '../lib/store';
import { PLUGIN_ID } from '../lib/constants';

function threadWithReminder(id: string, value: Record<string, unknown>) {
  const thread = new Thread({ id, accountId: 'a1' });
  thread.directlyAttachMetadata(PLUGIN_ID, value);
  return thread;
}

function focus(thread: Thread | null) {
  (FocusedContentStore.focused as unknown as jasmine.Spy).andReturn(thread);
  (SendRemindersStore as any)._onFocusedContentChanged();
}

describe('SendRemindersStore', function sendRemindersStore() {
  beforeEach(() => {
    (SendRemindersStore as any)._lastFocusedThread = null;
    spyOn(Actions, 'queueTask');
    spyOn(FocusedContentStore, 'focused').andReturn(null);
  });

  it('acknowledges a reminder once its thread loses focus', () => {
    const reminded = threadWithReminder('t1', { shouldNotify: true });
    focus(reminded);
    expect(Actions.queueTask).not.toHaveBeenCalled();
    focus(new Thread({ id: 't2', accountId: 'a1' }));
    const task = (Actions.queueTask as unknown as jasmine.Spy).calls[0].args[0];
    expect(task instanceof SyncbackMetadataTask).toBe(true);
    expect(task.pluginId).toBe(PLUGIN_ID);
    expect(task.modelId).toBe('t1');
    expect(task.value).toEqual({});
  });

  it('leaves a reminder that is not notifying alone', () => {
    focus(threadWithReminder('t1', { shouldNotify: false, expiration: new Date() }));
    focus(null);
    expect(Actions.queueTask).not.toHaveBeenCalled();
  });
});
