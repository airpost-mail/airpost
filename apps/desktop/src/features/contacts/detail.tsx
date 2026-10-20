import React from 'react';
import { Contact, localized, AccountStore, ContactGroup } from 'mailspring-exports';
import { isEqual } from 'underscore';
import { FocusContainer, ListensToFluxStore, ScrollRegion } from 'mailspring-component-kit';
import { ContactBase, ContactInteractorMetadata } from './types';
import { ContactDetailRead } from './detail-read';
import { ContactDetailEdit } from './detail-edit';
import { Store, ContactsPerspective } from './store';
import { getContact, saveContact } from '../../engine/client/pim';
import { Button } from '../../components/kit/button';

interface ContactDetailProps {
  editing: string | 'new' | false;
  groups: ContactGroup[];
  contacts: Contact[];
  perspective: ContactsPerspective;
  focusedId?: string;
}

interface ContactDetailState {
  contact: Contact | null;
  data: ContactBase | null;
  metadata: ContactInteractorMetadata | null;
}

// What a new contact starts from; contacts.create builds the vCard or Google person.
function emptyContactForAccountId(accountId: string): ContactDetailState {
  const account = AccountStore.accountForId(accountId);
  const google = account.provider === 'gmail';
  return {
    contact: new Contact({
      source: google ? 'gpeople' : 'carddav',
      name: '',
      email: '',
      accountId: account.id,
    }),
    data: {
      name: {
        displayName: '',
        familyName: '',
        givenName: '',
        honorificPrefix: '',
        honorificSuffix: '',
      },
      company: '',
      title: '',
      emailAddresses: [],
      phoneNumbers: [],
      addresses: [],
      urls: [],
      nicknames: [],
      relations: [],
      birthdays: [],
      notes: '',
    },
    metadata: { origin: google ? 'Google Contacts' : 'CardDAV', readonly: false },
  };
}

const EMPTY: ContactDetailState = { contact: null, data: null, metadata: null };

class ContactDetailWithFocus extends React.Component<ContactDetailProps, ContactDetailState> {
  // The contact whose details the last contacts.get was for, so a late answer is dropped.
  _loading: Contact | null = null;
  _mounted = false;

  constructor(props: ContactDetailProps) {
    super(props);
    this.state = EMPTY;
  }

  componentDidMount() {
    this._mounted = true;
    this.load();
  }

  componentWillUnmount() {
    this._mounted = false;
  }

  componentDidUpdate(prevProps: ContactDetailProps) {
    const prevContact = prevProps.contacts?.find((c) => c.id === prevProps.focusedId);
    const newContact = this.props.contacts?.find((c) => c.id === this.props.focusedId);

    if (isEqual(prevContact, newContact) && prevProps.editing === this.props.editing) return;
    if (newContact && this.props.editing !== newContact.id) Store.setEditing(false);
    this.load();
  }

  // The engine reads the vCard or Google person into the editor's shape (contacts.get).
  async load() {
    const { editing, contacts, focusedId, perspective } = this.props;
    if (editing === 'new' && 'accountId' in perspective) {
      this._loading = null;
      this.setState(emptyContactForAccountId(perspective.accountId));
      return;
    }
    const contact = contacts?.find((c) => c.id === focusedId);
    this._loading = contact || null;
    if (!contact) {
      this.setState(EMPTY);
      return;
    }
    try {
      const { data, metadata } = await getContact({
        accountId: contact.accountId,
        contactId: contact.id,
      });
      if (!this._mounted || this._loading !== contact) return;
      this.setState({ contact, data, metadata });
    } catch (err) {
      if (!this._mounted || this._loading !== contact) return;
      console.warn(`Contacts: could not read contact ${contact.id}: ${err.message}`);
      this.setState(EMPTY);
    }
  }

  onCancel = () => {
    Store.setEditing(false);
  };

  onSaveChanges = async () => {
    const { perspective } = this.props;
    const { contact, data } = this.state;
    if (!('accountId' in perspective) || !contact) return;

    try {
      await saveContact({
        accountId: contact.id ? contact.accountId : perspective.accountId,
        contactId: contact.id || undefined,
        data,
      });
      Store.setEditing(false);
    } catch (err) {
      AppEnv.showErrorDialog({
        title: localized('Unable to save contact'),
        message: err.message,
      });
    }
  };

  render() {
    const { editing, groups } = this.props;
    const { data, metadata, contact } = this.state;

    if (!data) {
      return (
        <div className="contact-detail-column">
          <div className="contacts-empty-state">
            <div className="message">No contact selected.</div>
          </div>
        </div>
      );
    }

    return (
      <>
        <ScrollRegion className="contact-detail-column">
          {editing ? (
            <ContactDetailEdit
              data={data}
              contact={contact}
              onChange={(changes) => this.setState({ data: { ...data, ...changes } })}
            />
          ) : (
            <ContactDetailRead data={data} contact={contact} metadata={metadata} groups={groups} />
          )}
        </ScrollRegion>
        {editing && (
          <div className="contact-edit-footer">
            <Button variant="outline" onClick={this.onCancel}>
              {localized('Cancel')}
            </Button>
            <div style={{ flex: 1 }} />
            <Button onClick={this.onSaveChanges}>{localized('Save Changes')}</Button>
          </div>
        )}
      </>
    );
  }
}

export const ContactDetail: React.FunctionComponent<ContactDetailProps> = ListensToFluxStore(
  ({ contacts, perspective, editing, groups }) => (
    <FocusContainer collection="contact">
      <ContactDetailWithFocus
        contacts={contacts}
        editing={editing}
        perspective={perspective}
        groups={groups}
      />
    </FocusContainer>
  ),
  {
    stores: [Store],
    getStateFromStores: () => ({
      editing: Store.editing(),
      groups: Store.groups(),
      contacts: Store.filteredContacts(),
      perspective: Store.perspective(),
    }),
  }
);

ContactDetail.displayName = 'ContactDetail';
(ContactDetail as any).containerStyles = {
  minWidth: 360,
  maxWidth: 100000,
};
