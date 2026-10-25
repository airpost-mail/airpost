import { FocusedContentStore, Thread } from 'mailspring-exports';
import BaseStore from 'mailspring-store';

import { PLUGIN_ID } from './constants';
import { updateReminderMetadata } from './utils';

// The engine keeps the reply baseline, clears a reminder on reply and reminds on expiry (its
// expiration executor); this window only acknowledges a reminder once the thread was read.
class SendRemindersStore extends BaseStore {
  _lastFocusedThread: Thread | null = null;
  _unsubscribers: (() => void)[] = [];

  activate() {
    this._unsubscribers = [FocusedContentStore.listen(this._onFocusedContentChanged)];
  }

  deactivate() {
    this._unsubscribers.forEach((unsub) => unsub());
  }

  _onFocusedContentChanged = () => {
    const thread = FocusedContentStore.focused('thread') || null;
    const last = this._lastFocusedThread;
    const didUnfocusLastThread = last && (!thread || thread.id !== last.id);
    // When we unfocus a thread that had `shouldNotify == true`, it means that
    // we have acknowledged the notification, or in this case, the reminder. If
    // that's the case, set `shouldNotify` to false.
    if (last && didUnfocusLastThread) {
      const metadata = last.metadataForPluginId(PLUGIN_ID);
      if (metadata && metadata.shouldNotify) {
        updateReminderMetadata(last, {});
      }
    }
    this._lastFocusedThread = thread;
  };
}

export default new SendRemindersStore();
