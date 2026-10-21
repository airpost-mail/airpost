import { localized } from '../../intl/localize';
import { Contact } from '../../data/models/contact';
import { Store } from './store';
import { exportContactsToFile } from './files';
import { showGPeopleReadonlyNotice } from './google-support';
import { Menus } from '../../platform/app-api';

type TemplateItem = { label: string; click: () => void } | { type: 'separator' };

export class ContactListContextMenu {
  contacts: Contact[];

  constructor(contacts: Contact[]) {
    this.contacts = contacts;
  }

  editItem(): TemplateItem | null {
    if (this.contacts.length !== 1) return null;
    const contact = this.contacts[0];
    if (contact.source === 'mail') return null;
    return {
      label: localized('Edit'),
      click: () => {
        if (showGPeopleReadonlyNotice(contact.accountId)) return;
        Store.setEditing(contact.id);
      },
    };
  }

  exportItem(): TemplateItem {
    const count = this.contacts.length;
    return {
      label:
        count === 1
          ? localized('Export vCard...')
          : localized('Export %@ vCards...', String(count)),
      click: () => exportContactsToFile(this.contacts),
    };
  }

  template(): TemplateItem[] {
    const items: (TemplateItem | null)[] = [
      this.editItem(),
      { type: 'separator' },
      this.exportItem(),
    ];

    // Drop nulls, then strip any separators at the leading or trailing edges.
    return items
      .filter((item): item is TemplateItem => !!item)
      .filter((item, idx, arr) => {
        if (!('type' in item)) return true;
        return idx !== 0 && idx !== arr.length - 1;
      });
  }

  displayMenu() {
    Menus.popup(this.template());
  }
}
