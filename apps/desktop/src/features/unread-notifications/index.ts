import { Thread } from '../../data/models/thread';
import * as Actions from '../../data/actions';
import { AccountStore } from '../../data/stores/account';
import { Message } from '../../data/models/message';
import SoundRegistry from '../../packages/sound-registry';
import NativeNotifications from '../../window/platform/native-notifications';
import DatabaseStore from '../../data/database/store';
import { localized } from '../../intl/localize';
import { DatabaseChangeRecord } from '../../data/database/change-record';
import { TaskFactory } from '../../data/tasks/factory';
import EngineEvents, { EngineEvents as EngineEventsType } from '../../engine/client/events';
import { Feature, MailNewParams } from '../../engine/protocol';

const WAIT_FOR_CHANGES_DELAY = 400;
const NEW_MAIL_SOUND_QUIET_PERIOD = 5000;
// mail.new arrives once per message; the ones this close together are handled as one batch,
// as the messages of one delta were.
const MAIL_NEW_BATCH_DELAY = 100;
// How many notified message ids are remembered to drop a repeated mail.new.
const MAIL_NEW_SEEN_LIMIT = 2000;

export class Notifier {
  activationTime = Date.now();
  unnotifiedQueue = [];
  hasScheduledNotify = false;
  lastNewMailSoundRequest = 0;
  unlisteners: Array<() => void>;
  activeNotifications = {};
  engine: EngineEventsType;
  mailNewBatch: MailNewParams[] = [];
  mailNewTimer = null;
  mailNewSeen = new Set<string>();

  constructor(engine: EngineEventsType = EngineEvents) {
    this.engine = engine;
    engine.on('mail-new', this._onMailNew);
    this.unlisteners = [
      DatabaseStore.listen(this._onDatabaseChanged, this),
      () => engine.removeListener('mail-new', this._onMailNew),
      () => clearTimeout(this.mailNewTimer),
    ];
  }

  unlisten() {
    for (const unlisten of this.unlisteners) {
      unlisten();
    }
  }

  // async for testing
  async _onDatabaseChanged({ objectClass, objects, objectsRawJSON }: DatabaseChangeRecord<any>) {
    if (AppEnv.config.get('core.notifications.enabled') === false) {
      return;
    }

    if (objectClass === Thread.name) {
      this._onThreadsChanged(objects);
    }

    // An engine with mail.new decides what is new mail; see _onMailNew.
    if (objectClass === Message.name && !this.engine.has(Feature.MailNew)) {
      const newIds = new Set<string>();
      for (const json of objectsRawJSON) {
        if (json.headersSyncComplete) newIds.add(json.id);
      }
      if (!newIds.size) return;
      this._onMessagesChanged(objects, newIds);
    }
  }

  _onMailNew = (params: MailNewParams) => {
    if (!params || !params.messageId || this.mailNewSeen.has(params.messageId)) return;
    if (this.mailNewSeen.size >= MAIL_NEW_SEEN_LIMIT) this.mailNewSeen.clear();
    this.mailNewSeen.add(params.messageId);
    this.mailNewBatch.push(params);
    if (!this.mailNewTimer) {
      this.mailNewTimer = setTimeout(() => {
        this.mailNewTimer = null;
        this._onMailNewBatch();
      }, MAIL_NEW_BATCH_DELAY);
    }
  };

  // async for testing. The engine checked unread, inbox, the first headers sync and the
  // sender; the launch time and the 400 ms wait for later changes still apply here.
  async _onMailNewBatch() {
    const batch = this.mailNewBatch;
    this.mailNewBatch = [];
    if (!batch.length || AppEnv.config.get('core.notifications.enabled') === false) {
      return;
    }
    const ids = batch.map((p) => p.messageId);
    const msgs = await DatabaseStore.findAll<Message>(Message, Message.attributes.id.in(ids));
    if (!msgs.length) return;
    await this._onMessagesChanged(msgs, new Set(ids));
  }

  // async for testing
  async _onMessagesChanged(msgs, newIds: Set<string>) {
    const notifworthy = {};

    for (const msg of msgs) {
      // ensure the message is unread
      if (msg.unread !== true) continue;
      // ensure the message was just created (eg: this is not a modification).
      // The sync engine attaches a JSON key to let us know that this is the first
      // message emitted about this Message. (Hooray hacks around reactive patterns)
      if (!newIds.has(msg.id)) continue;
      // ensure the message was received after the app launched (eg: not syncing an old email)
      if (!msg.date || msg.date.valueOf() < this.activationTime) continue;
      // ensure the message is not a loopback
      const account = msg.from[0] && AccountStore.accountForEmail(msg.from[0].email);
      if (msg.accountId === (account || { id: undefined }).id) continue;

      notifworthy[msg.id] = msg;
    }

    if (Object.keys(notifworthy).length === 0) {
      return;
    }

    if (!AppEnv.inSpecMode()) {
      await new Promise<void>((resolve) => {
        // wait a couple hundred milliseconds and collect any updates to these
        // new messages. This gets us message bodies, messages impacted by mail rules, etc.
        // while ensuring notifications are never too delayed.
        const unlisten = DatabaseStore.listen(({ objectClass, objects }) => {
          if (objectClass !== Message.name) {
            return;
          }
          for (const msg of objects) {
            if (notifworthy[msg.id]) {
              notifworthy[msg.id] = msg;
              if (msg.unread === false) {
                delete notifworthy[msg.id];
              }
            }
          }
        });
        setTimeout(() => {
          unlisten();
          resolve();
        }, WAIT_FOR_CHANGES_DELAY);
      });
    }

    await this._onNewMessagesReceived(Object.values(notifworthy));
  }

  _onThreadsChanged(threads) {
    // Ensure notifications are dismissed when the user reads a thread
    threads.forEach(({ id, unread }) => {
      if (!unread && this.activeNotifications[id]) {
        this.activeNotifications[id].forEach((n) => n.close());
        delete this.activeNotifications[id];
      }
    });
  }

  async _notifyAll(playSound: boolean) {
    // Extract unique sender names from the queue
    const count = this.unnotifiedQueue.length;
    const senders = [
      ...new Set(
        this.unnotifiedQueue
          .map(({ message }) => (message.from[0] ? message.from[0].displayName() : null))
          .filter(Boolean)
      ),
    ] as string[];

    try {
      await NativeNotifications.displaySummaryNotification({
        count,
        senders,
        playSound,
        onActivate: () => {
          AppEnv.displayWindow();
        },
      });
    } finally {
      // Only remove the items we counted — new items may have been queued during the await.
      // Removed on failure too, so a notification that throws isn't retried every 2s forever.
      this.unnotifiedQueue.splice(0, count);
    }
  }

  async _notifyOne({ message, thread }, playSound: boolean) {
    const from = message.from[0] ? message.from[0].displayName() : 'Unknown';
    const title = from;
    let subtitle = null;
    let body = null;
    if (message.subject && message.subject.length > 0) {
      subtitle = message.subject;
      body = message.snippet;
    } else {
      subtitle = message.snippet;
      body = null;
    }

    const notification = await NativeNotifications.displayNotification({
      title: title,
      subtitle: subtitle,
      body: body,
      tag: `thread-${thread.id}`,
      threadId: thread.id,
      messageId: message.id,
      playSound,

      // macOS inline reply
      canReply: true,
      replyPlaceholder: localized('Reply to %@...', from),

      // macOS action buttons
      actions: [
        { type: 'button', text: localized('Mark as Read') },
        { type: 'button', text: localized('Archive') },
      ],

      onActivate: ({ response, activationType, actionIndex }) => {
        if (activationType === 'replied' && response && typeof response === 'string') {
          Actions.sendQuickReply({ thread, message }, response);
        } else if (activationType === 'action') {
          this._handleNotificationAction(actionIndex, thread);
        } else if (activationType === 'clicked') {
          AppEnv.displayWindow();
          if (!thread) {
            AppEnv.showErrorDialog(`Can't find that thread`);
            return;
          }
          Actions.ensureCategoryIsFocused('inbox', thread.accountId);
          Actions.setFocus({ collection: 'thread', item: thread });
        }
      },
    });

    if (notification) {
      if (!this.activeNotifications[thread.id]) {
        this.activeNotifications[thread.id] = [notification];
      } else {
        this.activeNotifications[thread.id].push(notification);
      }
    }
  }

  _handleNotificationAction(actionIndex: number, thread: Thread) {
    switch (actionIndex) {
      case 0: // Mark as Read
        Actions.queueTask(
          TaskFactory.taskForSettingUnread({
            threads: [thread],
            unread: false,
            source: 'Notification Action',
          })
        );
        break;
      case 1: // Archive
        Actions.queueTasks(
          TaskFactory.tasksForArchiving({
            threads: [thread],
            source: 'Notification Action',
          })
        );
        break;
    }
  }

  async _notifyMessages(playSound = false) {
    // Set the guard immediately to prevent concurrent re-entry during async operations.
    // Without this, a second _onNewMessagesReceived call during the await below would
    // start a concurrent _notifyMessages, causing duplicate or conflicting notifications.
    this.hasScheduledNotify = true;

    // A failure must not skip the guard reset below, or notifications stop until relaunch.
    try {
      if (this.unnotifiedQueue.length >= 5) {
        await this._notifyAll(playSound);
      } else if (this.unnotifiedQueue.length > 0) {
        await this._notifyOne(this.unnotifiedQueue.shift(), playSound);
      }
    } catch (err) {
      AppEnv.reportError(err);
    }

    if (this.unnotifiedQueue.length > 0) {
      setTimeout(() => this._notifyMessages(), 2000);
    } else {
      this.hasScheduledNotify = false;
    }
  }

  // One sound per burst of mail: suppressed until mail has stopped arriving for the quiet period.
  _shouldPlayNewMailSound() {
    const now = Date.now();
    const quiet = now - this.lastNewMailSoundRequest >= NEW_MAIL_SOUND_QUIET_PERIOD;
    this.lastNewMailSoundRequest = now;
    return quiet && !!AppEnv.config.get('core.notifications.sounds');
  }

  _onNewMessagesReceived(newMessages) {
    if (newMessages.length === 0) {
      return Promise.resolve();
    }

    // For each message, find it's corresponding thread. First, look to see
    // if it's already in the `incoming` payload (sent via delta sync
    // at the same time as the message.) If it's not, try loading it from
    // the local cache.

    const threadIds = {};
    for (const { threadId } of newMessages) {
      threadIds[threadId] = true;
    }

    // TODO: Use xGMLabels + folder on message to identify which ones
    // are in the inbox to avoid needing threads here.
    return DatabaseStore.findAll<Thread>(
      Thread,
      Thread.attributes.id.in(Object.keys(threadIds))
    ).then((threadsArray) => {
      const threads = {};
      for (const t of threadsArray) {
        threads[t.id] = t;
      }

      // Filter new messages to just the ones in the inbox
      const newMessagesInInbox = newMessages.filter(({ threadId }) => {
        return threads[threadId] && threads[threadId].categories.find((c) => c.role === 'inbox');
      });

      if (newMessagesInInbox.length === 0) {
        return;
      }

      for (const msg of newMessagesInInbox) {
        this.unnotifiedQueue.push({ message: msg, thread: threads[msg.threadId] });
      }
      if (!this.hasScheduledNotify) {
        const playSound = this._shouldPlayNewMailSound();
        if (playSound && !NativeNotifications.playsSoundWithNotification) {
          SoundRegistry.playSound('new-mail');
        }
        this._notifyMessages(playSound && NativeNotifications.playsSoundWithNotification);
      }
    });
  }
}

export const config = {
  enabled: {
    type: 'boolean',
    default: true,
  },
};

export function activate() {
  this.notifier = new Notifier();
}

export function deactivate() {
  this.notifier.unlisten();
}
