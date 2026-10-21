import { SyncbackEventTask } from '../../src/data/tasks/syncback-event';
import { DestroyEventTask } from '../../src/data/tasks/destroy-event';

describe('re-reading the calendar after an event write', () => {
  beforeEach(() => {
    spyOn(AppEnv.engineBridge, 'sendSyncCalendarNow');
  });

  it('polls the account a saved event belongs to once the server has it', async () => {
    const task = new SyncbackEventTask({ accountId: 'account-1' } as any);

    await task.onSuccess();

    expect(AppEnv.engineBridge.sendSyncCalendarNow).toHaveBeenCalledWith('account-1');
  });

  it('polls the account a deleted event belonged to once the server has the deletion', async () => {
    const task = new DestroyEventTask({ accountId: 'account-2' } as any);

    await task.onSuccess();

    expect(AppEnv.engineBridge.sendSyncCalendarNow).toHaveBeenCalledWith('account-2');
  });
});
