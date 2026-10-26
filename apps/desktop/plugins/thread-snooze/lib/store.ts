import BaseStore from 'mailspring-store';

import { Thread, UndoRedoStore, localized } from 'mailspring-exports';
import { intentErrorMessage, snoozeThreads } from '../../../src/engine/client/intents';

import { snoozedUntilMessage } from './utils';
import * as SnoozeActions from './actions';

// Snoozing is the engine's `threads.snooze`: it moves each account's threads to Snoozed and
// writes their `expiration`, and keeps the undo entry; its expiration executor unsnoozes them
// when it comes, and resurfaces them (core.notifications.unsnoozeToTop).
class _SnoozeStore extends BaseStore {
  unsubscribers: Array<() => void> = [];

  activate() {
    this.unsubscribers = [SnoozeActions.snoozeThreads.listen(this._onSnoozeThreads)];
  }

  deactivate() {
    this.unsubscribers.forEach((unsub) => unsub());
  }

  _onSnoozeThreads = async (threads: Thread[], snoozeDate: string, label: string) => {
    const until = Math.floor(new Date(snoozeDate).getTime() / 1000);
    const byAccount = new Map<string, string[]>();
    for (const t of threads) {
      byAccount.set(t.accountId, [...(byAccount.get(t.accountId) || []), t.id]);
    }
    for (const [accountId, threadIds] of byAccount) {
      try {
        const { undo } = await snoozeThreads({ accountId, threadIds, until });
        UndoRedoStore.describe(undo, snoozedUntilMessage(snoozeDate));
      } catch (error) {
        AppEnv.showErrorDialog(
          localized(
            'Sorry, we were unable to save your snooze settings. %@',
            intentErrorMessage(error)
          )
        );
        return;
      }
    }
  };
}

export const SnoozeStore = new _SnoozeStore();
