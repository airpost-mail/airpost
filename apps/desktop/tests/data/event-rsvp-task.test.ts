import { EventRSVPTask } from '../../src/data/tasks/event-rsvp';
import { Message } from '../../src/data/models/message';
import * as Actions from '../../src/data/actions';
import DatabaseStore from '../../src/data/database/store';
import { SyncbackMetadataTask } from '../../src/data/tasks/syncback-metadata';

const ACCOUNT_ID = 'acct-1';

describe('EventRSVPTask', () => {
  let queueTask: jasmine.Spy;

  beforeEach(() => {
    spyOn(AppEnv.engineBridge, 'sendSyncCalendarNow');
    queueTask = spyOn(Actions, 'queueTask');
  });

  it('syncs the account calendar after a response completes', async () => {
    const task = new EventRSVPTask({ accountId: 'account-1' } as any);

    await task.onSuccess();

    expect(AppEnv.engineBridge.sendSyncCalendarNow).toHaveBeenCalledWith('account-1');
  });

  it('records the response on the invitation message before syncing', async () => {
    const message = new Message({ id: 'message-1', accountId: 'account-1' });
    spyOn(DatabaseStore, 'find').andReturn(Promise.resolve(message));
    const task = new EventRSVPTask({
      accountId: 'account-1',
      messageId: 'message-1',
      icsRSVPStatus: 'ACCEPTED',
    } as any);

    await task.onSuccess();

    const queued = queueTask.mostRecentCall.args[0];
    expect(queued instanceof SyncbackMetadataTask).toBe(true);
    expect(queued.value.status).toBe('ACCEPTED');
    expect(AppEnv.engineBridge.sendSyncCalendarNow).toHaveBeenCalledWith('account-1');
  });

  it('still syncs when the invitation message is no longer available', async () => {
    spyOn(DatabaseStore, 'find').andReturn(Promise.resolve(null));
    const task = new EventRSVPTask({
      accountId: 'account-1',
      messageId: 'missing',
      icsRSVPStatus: 'DECLINED',
    } as any);

    await task.onSuccess();

    expect(queueTask).not.toHaveBeenCalled();
    expect(AppEnv.engineBridge.sendSyncCalendarNow).toHaveBeenCalledWith('account-1');
  });
});

describe('EventRSVPTask.forProposingNewTime', function () {
  const propose = () =>
    EventRSVPTask.forProposingNewTime({
      accountId: ACCOUNT_ID,
      to: 'ada@example.com',
      messageId: 'message-1',
      ics: 'BEGIN:VCALENDAR\r\nMETHOD:COUNTER\r\nEND:VCALENDAR',
      summary: 'Standup',
      comment: 'Clashes with my review',
    });

  it('is a COUNTER addressed to the organizer', function () {
    const task = propose();
    expect(task.method).toBe('COUNTER');
    expect((task as any).to).toBe('ada@example.com');
    expect(task.subject).toContain('Standup');
    expect(task.comment).toBe('Clashes with my review');
  });

  it('says what it is doing', function () {
    expect(propose().label()).toBe('Proposing a new time');
    expect(new EventRSVPTask({ accountId: ACCOUNT_ID, method: 'REPLY' } as any).label()).toBe(
      'Sending RSVP'
    );
  });

  it('does not record an answer on the invitation, since a proposal is not one', async function () {
    spyOn(AppEnv.engineBridge, 'sendSyncCalendarNow');
    const queueTask = spyOn(Actions, 'queueTask');
    const find = spyOn(DatabaseStore, 'find');
    const task = propose();
    (task as any).icsRSVPStatus = 'TENTATIVE'; // even if a status were set

    await task.onSuccess();

    expect(find).not.toHaveBeenCalled();
    expect(queueTask).not.toHaveBeenCalled();
    expect(AppEnv.engineBridge.sendSyncCalendarNow).toHaveBeenCalledWith(ACCOUNT_ID);
  });
});
