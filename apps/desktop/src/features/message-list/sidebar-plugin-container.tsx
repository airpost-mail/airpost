import React from 'react';
import { localized } from '../../intl/localize';
import FocusedContactsStore from '../../data/stores/focused-contacts';
import { Contact } from '../../data/models/contact';
import { InjectedComponentSet } from '../../components/injected/component-set';

class FocusedContactStorePropsContainer extends React.Component<
  { children: React.ReactElement<any> },
  { sortedContacts: Contact[]; focusedContact: Contact | null | undefined }
> {
  static displayName = 'FocusedContactStorePropsContainer';

  unsubscribe?: () => void;

  constructor(props: { children: React.ReactElement<any> }) {
    super(props);
    this.state = this._getStateFromStores();
  }

  componentDidMount() {
    this.unsubscribe = FocusedContactsStore.listen(this._onChange);
  }

  componentWillUnmount() {
    this.unsubscribe?.();
  }

  _onChange = () => {
    this.setState(this._getStateFromStores());
  };

  _getStateFromStores() {
    return {
      sortedContacts: FocusedContactsStore.sortedContacts(),
      focusedContact: FocusedContactsStore.focusedContact(),
    };
  }

  render() {
    let classname = 'sidebar-section';
    let inner = null;
    if (this.state.focusedContact) {
      classname += ' visible';
      inner = React.cloneElement(this.props.children, this.state);
    }
    return (
      <aside className={classname} aria-label={localized('Contact information')}>
        {inner}
      </aside>
    );
  }
}

// Rendered by FocusedContactStorePropsContainer only when there is a focused contact.
const SidebarPluginContainerInner = (props: { focusedContact?: Contact | null }) => {
  if (!props.focusedContact) {
    return null;
  }
  return (
    <InjectedComponentSet
      className="sidebar-contact-card"
      key={props.focusedContact.email}
      matching={{ role: 'MessageListSidebar:ContactCard' }}
      direction="column"
      exposedProps={{
        contact: props.focusedContact,
      }}
    />
  );
};

export class SidebarPluginContainer extends React.Component {
  static displayName = 'SidebarPluginContainer';

  static containerStyles = {
    order: 1,
    flexShrink: 0,
    minWidth: 150,
    maxWidth: 300,
  };

  render() {
    return (
      <FocusedContactStorePropsContainer>
        <SidebarPluginContainerInner />
      </FocusedContactStorePropsContainer>
    );
  }
}
