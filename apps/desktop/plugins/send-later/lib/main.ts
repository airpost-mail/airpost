import {
  ComponentRegistry,
  DatabaseStore,
  DraftStore,
  Message,
  SyncbackMetadataTask,
  SendActionsStore,
  Actions,
  localized,
} from 'mailspring-exports';
import { HasTutorialTip } from 'mailspring-component-kit';

import EngineEvents from '../../../src/engine/client/events';
import { Feature } from '../../../src/engine/protocol';
import SendLaterButton from './button';
import SendLaterStatus from './status';
import { PLUGIN_ID } from './constants';

let unlisten: (() => void) | null = null;

const SendLaterButtonWithTip = HasTutorialTip(SendLaterButton, {
  title: localized('Send on your own schedule'),
  instructions: localized(
    'Schedule this message to send at the ideal time. Tern makes it easy to control the fabric of spacetime!'
  ),
});

/** The fields of a DatabaseStore change record this reads; the objects are Messages once the class matches. */
interface ExpirationChange {
  type: string;
  objectClass: string;
  objects: Message[];
}

export function handleMetadataExpiration(change: ExpirationChange, engine = EngineEvents) {
  if (change.type !== 'metadata-expiration' || change.objectClass !== Message.name) {
    return;
  }

  change.objects.forEach(async (message: Message) => {
    const metadata = message.metadataForPluginId(PLUGIN_ID);
    if (!metadata || !metadata.expiration || metadata.expiration > new Date()) {
      return;
    }
    // The engine sends it, unless the send needs this window's transforms or a send action
    // only a plugin has. It also clears the schedule of a message that is no longer a draft.
    if (engine.has(Feature.ExpirationExecutor) && !metadata.clientFinalize) {
      const action = metadata.actionKey || SendActionsStore.DefaultSendActionKey;
      if (!message.draft || SendActionsStore.enginePerformsSendAction(action)) {
        return;
      }
    }

    if (!message.draft) {
      Actions.queueTask(
        SyncbackMetadataTask.forSaving({
          model: message,
          pluginId: PLUGIN_ID,
          value: {
            expiration: null,
          },
        })
      );
      return;
    }

    // clear the metadata
    const session = await DraftStore.sessionForClientId(message.headerMessageId);
    if (!session.draft()) {
      // Draft was already sent or deleted before the expiration fired
      return;
    }
    session.changes.addPluginMetadata(PLUGIN_ID, { expiration: null });
    session.changes.commit();

    // send the draft
    const actionKey = metadata.actionKey || SendActionsStore.DefaultSendActionKey;
    Actions.sendDraft(message.headerMessageId, { actionKey, delay: 0 });
  });
}

export function activate() {
  ComponentRegistry.register(SendLaterButtonWithTip, { role: 'Composer:ActionButton' });
  ComponentRegistry.register(SendLaterStatus, { role: 'DraftList:DraftStatus' });

  if (AppEnv.isMainWindow()) {
    unlisten = DatabaseStore.listen(handleMetadataExpiration);
  }
}

export function deactivate() {
  ComponentRegistry.unregister(SendLaterButtonWithTip);
  ComponentRegistry.unregister(SendLaterStatus);
  if (unlisten) {
    unlisten();
  }
}

export function serialize() {}
