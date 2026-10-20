import { Actions, Folder, Thread, SyncbackMetadataTask } from 'mailspring-exports';
import { SnoozeStore } from '../lib/store';
import { PLUGIN_ID } from '../lib/constants';
import { EngineEvents } from '../../../src/engine/events';
import { Feature } from '../../../src/shared/engine/protocol';

function snoozedThread(id: string, expiration: Date) {
  const snoozed = new Folder({ id: `${id}-folder`, accountId: 'a1', role: 'snoozed' });
  return new Thread({ id, accountId: 'a1', folders: [snoozed] }).directlyAttachMetadata(PLUGIN_ID, {
    expiration,
  }) as Thread;
}

describe('SnoozeStore metadata expiration', function () {
  beforeEach(() => {
    spyOn(AppEnv, 'isMainWindow').andReturn(true);
    spyOn(Actions, 'queueTasks');
    spyOn(SnoozeStore, '_onUnsnoozeThreads');
  });

  it('clears the expiration of the due threads only, when a batch also holds a later snooze', () => {
    const due = snoozedThread('due', new Date(Date.now() - 60 * 1000));
    const later = snoozedThread('later', new Date(Date.now() + 24 * 3600 * 1000));

    SnoozeStore._onMetadataExpired([due, later]);

    expect(Actions.queueTasks).toHaveBeenCalled();
    const tasks = (Actions.queueTasks as unknown as jasmine.Spy).calls[0]
      .args[0] as SyncbackMetadataTask[];
    expect(tasks.map((t) => t.modelId)).toEqual(['due']);
    expect(tasks[0].pluginId).toBe(PLUGIN_ID);
    expect(tasks[0].value.expiration).toBe(null);

    const unsnoozed = (SnoozeStore._onUnsnoozeThreads as jasmine.Spy).calls[0].args[0] as Thread[];
    expect(unsnoozed.map((t) => t.id)).toEqual(['due']);
  });

  it('queues nothing when no thread in the batch is due', () => {
    SnoozeStore._onMetadataExpired([snoozedThread('later', new Date(Date.now() + 3600 * 1000))]);
    expect(Actions.queueTasks).not.toHaveBeenCalled();
    expect(SnoozeStore._onUnsnoozeThreads).not.toHaveBeenCalled();
  });

  it('leaves due threads to an engine that runs the expiration executor', () => {
    const engine = new EngineEvents();
    engine.connected([Feature.ExpirationExecutor], () => Promise.resolve(null));
    SnoozeStore._onMetadataExpired(
      [snoozedThread('due', new Date(Date.now() - 60 * 1000))],
      engine
    );
    expect(Actions.queueTasks).not.toHaveBeenCalled();
    expect(SnoozeStore._onUnsnoozeThreads).not.toHaveBeenCalled();
  });
});
