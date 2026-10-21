import {
  Actions,
  FocusedContentStore,
  SendDraftTask,
  DatabaseStore,
  Thread,
  DraftFactory,
  DatabaseChangeRecord,
} from 'mailspring-exports';
import BaseStore from 'mailspring-store';

import EngineEvents from '../../../src/engine/client/events';
import { Feature } from '../../../src/engine/protocol';
import { resurfacingBodies } from '../../../src/engine/resurfacing';
import { PLUGIN_ID } from './constants';
import { updateReminderMetadata } from './utils';

class SendRemindersStore extends BaseStore {
  _lastFocusedThread: Thread | null = null;
  _unsubscribers: (() => void)[] = [];

  activate() {
    this._unsubscribers = [
      FocusedContentStore.listen(this._onFocusedContentChanged),
      DatabaseStore.listen(this._onDatabaseChanged),
    ];
  }

  deactivate() {
    this._unsubscribers.forEach((unsub) => unsub());
  }

  _sendReminderEmail = async (thread: Thread, sentHeaderMessageId: string) => {
    const body = resurfacingBodies().reminderBody;

    const draft = await DraftFactory.createDraftForResurfacing(thread, sentHeaderMessageId, body);
    Actions.queueTask(SendDraftTask.forSending(draft, { silent: true }));
  };

  _onDatabaseChanged = (
    { type, objects, objectClass }: DatabaseChangeRecord<Thread>,
    engine = EngineEvents
  ) => {
    if (objectClass !== Thread.name) {
      return;
    }

    // The engine keeps the baseline, clears on reply and reminds when it runs the expiration
    // executor; the focus handler below still acknowledges a reminder.
    if (!AppEnv.isMainWindow() || engine.has(Feature.ExpirationExecutor)) {
      return;
    }

    // If threads with reminders were deleted, there's nothing to clean up — the
    // metadata is gone with the thread. Just skip processing to avoid operating
    // on deleted models.
    if (type === 'unpersist') {
      return;
    }

    for (const thread of objects) {
      const metadata = thread.metadataForPluginId(PLUGIN_ID);
      if (!metadata || !metadata.expiration) {
        continue;
      }

      const currentReplyTimestamp = new Date(thread.lastMessageReceivedTimestamp).getTime() / 1000;

      // Metadata the sync engine promoted from a draft on send carries only
      // {expiration, sentHeaderMessageId}. Record the reply baseline the first
      // time we see it; comparing the missing field below would read as "a reply
      // arrived" and silently discard the reminder.
      if (metadata.lastReplyTimestamp === undefined) {
        metadata.lastReplyTimestamp = currentReplyTimestamp;
        metadata.shouldNotify = false;
        if (type !== 'metadata-expiration') {
          updateReminderMetadata(thread, metadata);
          continue;
        }
      }

      // has a new message arrived on the thread? if so, clear the metadata completely
      if (metadata.lastReplyTimestamp !== currentReplyTimestamp) {
        updateReminderMetadata(thread, {});
        continue;
      }

      // has the metadata expired? If so, send the reminder email and
      // advance metadata into the "notify" phase.
      if (type === 'metadata-expiration' && metadata.expiration <= new Date()) {
        // mark that the email should enter the notification highlight state
        updateReminderMetadata(thread, { ...metadata, expiration: null, shouldNotify: true });
        // send an email on the thread, causing the thread to move up in the inbox
        this._sendReminderEmail(thread, metadata.sentHeaderMessageId);
      }
    }
  };

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
