import * as React from 'react';
import { Message } from '../../data/models/message';
import { File } from '../../data/models/file';
import { MessageViewExtension } from '../../packages/message-view-extension';
import * as ExtensionRegistry from '../../packages/extension-registry';
import ComponentRegistry from '../../packages/component-registry';
import DatabaseStore from '../../data/database/store';
import { EventHeader } from './header';
import { bestICSAttachment } from './ics-attachment';
import { InvitationCalendarRefresh } from './invitation-calendar-refresh';

let stopRefreshingOnInvitations: (() => void) | null = null;

const EventHeaderContainer: React.FunctionComponent<{ message: Message }> = ({ message }) => {
  const icsFile = bestICSAttachment(message.files);
  return icsFile ? <EventHeader key={icsFile.id} message={message} file={icsFile} /> : null;
};

EventHeaderContainer.displayName = 'EventHeaderContainer';

class HideICSAttachmentExtension extends MessageViewExtension {
  static filterMessageFiles({ message, files }: { message: Message; files: File[] }): File[] {
    const best = bestICSAttachment(message.files);
    if (!best) return files;

    // Many automatic invite emails attach the ICS file more than once using different mimetypes
    // to ensure it's recognized everywhere, so we remove all the attachments with the exact
    // file size and an ics content type.
    return files.filter(
      (f) => !(f.size === best.size && ['application/ics', 'text/calendar'].includes(f.contentType))
    );
  }
}

export function activate() {
  ExtensionRegistry.MessageView.register(HideICSAttachmentExtension);
  ComponentRegistry.register(EventHeaderContainer, { role: 'message:BodyHeader' });
  // Every window sees each delta, so only the main window refreshes, once per invitation.
  if (AppEnv.isMainWindow()) {
    const refresher = new InvitationCalendarRefresh((accountId) =>
      AppEnv.engineBridge.sendSyncCalendarNow(accountId)
    );
    stopRefreshingOnInvitations = DatabaseStore.listen(refresher.onDatabaseChanged);
  }
}

export function deactivate() {
  ExtensionRegistry.MessageView.unregister(HideICSAttachmentExtension);
  ComponentRegistry.unregister(EventHeaderContainer);
  stopRefreshingOnInvitations?.();
}
