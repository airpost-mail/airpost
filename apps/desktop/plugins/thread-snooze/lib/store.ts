import BaseStore from 'mailspring-store';

import { SyncbackMetadataTask, Actions, Thread } from 'mailspring-exports';

import { moveThreads, snoozedUntilMessage } from './utils';
import { PLUGIN_ID } from './constants';
import * as SnoozeActions from './actions';

// Snoozing moves the threads and writes their `expiration`; the engine's expiration executor
// unsnoozes them when it comes, and resurfaces them (core.notifications.unsnoozeToTop).
class _SnoozeStore extends BaseStore {
  unsubscribers: Array<() => void> = [];

  activate() {
    this.unsubscribers = [SnoozeActions.snoozeThreads.listen(this._onSnoozeThreads)];
  }

  deactivate() {
    this.unsubscribers.forEach((unsub) => unsub());
  }

  _onSnoozeThreads = async (threads: Thread[], snoozeDate: string, label: string) => {
    try {
      // move the threads to the snoozed folder
      await moveThreads(threads, {
        snooze: true,
        description: snoozedUntilMessage(snoozeDate),
      });

      // attach metadata to the threads to unsnooze them later
      Actions.queueTasks(
        threads.map((model) =>
          SyncbackMetadataTask.forSaving({
            model,
            pluginId: PLUGIN_ID,
            value: {
              expiration: snoozeDate,
            },
          })
        )
      );
    } catch (error) {
      moveThreads(threads, { snooze: false, description: 'Unsnoozed' });
      Actions.closePopover();
      AppEnv.reportError(error);
      AppEnv.showErrorDialog(
        `Sorry, we were unable to save your snooze settings. ${(error as Error).message}`
      );
    }
  };
}

export const SnoozeStore = new _SnoozeStore();
