import { localized, ComponentRegistry, ExtensionRegistry } from 'mailspring-exports';
import { HasTutorialTip } from 'mailspring-component-kit';
import SendRemindersThreadTimestamp from './thread-timestamp';
import SendRemindersComposerButton from './composer-button';
import SendRemindersToolbarButton from './toolbar-button';
import { ScheduledReminderThreadHeader, NotificationExplanationMessageHeader } from './headers';
import SendRemindersStore from './store';
import * as ThreadListExtension from './thread-list-extension';
import * as AccountSidebarExtension from './account-sidebar-extension';

const ComposerButtonWithTip = HasTutorialTip(SendRemindersComposerButton, {
  title: localized('Get reminded!'),
  instructions: localized(
    "Get reminded if you don't receive a reply for this message within a specified time."
  ),
});

export function activate() {
  ComponentRegistry.register(ComposerButtonWithTip, { role: 'Composer:ActionButton' });
  ComponentRegistry.register(SendRemindersToolbarButton, { role: 'ThreadActionsToolbarButton' });
  ComponentRegistry.register(SendRemindersThreadTimestamp, { role: 'ThreadListTimestamp' });
  ComponentRegistry.register(NotificationExplanationMessageHeader, { role: 'MessageHeader' });
  ComponentRegistry.register(ScheduledReminderThreadHeader, { role: 'MessageListHeaders' });
  ExtensionRegistry.ThreadList.register(ThreadListExtension);
  ExtensionRegistry.AccountSidebar.register(AccountSidebarExtension);
  SendRemindersStore.activate();
}

export function deactivate() {
  ComponentRegistry.unregister(ComposerButtonWithTip);
  ComponentRegistry.unregister(SendRemindersToolbarButton);
  ComponentRegistry.unregister(SendRemindersThreadTimestamp);
  ComponentRegistry.unregister(NotificationExplanationMessageHeader);
  ComponentRegistry.unregister(ScheduledReminderThreadHeader);
  ExtensionRegistry.ThreadList.unregister(ThreadListExtension);
  ExtensionRegistry.AccountSidebar.unregister(AccountSidebarExtension);
  SendRemindersStore.deactivate();
}
