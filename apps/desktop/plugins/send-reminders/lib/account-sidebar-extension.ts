import { localized } from 'mailspring-exports';
import SendRemindersMailboxPerspective from './mailbox-perspective';

export const name = 'SendRemindersAccountSidebarExtension';

export function sidebarItem(accountIds: string[]) {
  return {
    id: 'Reminders',
    name: localized('Reminders'),
    iconName: 'reminders.png',
    perspective: new SendRemindersMailboxPerspective(accountIds),
    insertAtTop: true,
  };
}
