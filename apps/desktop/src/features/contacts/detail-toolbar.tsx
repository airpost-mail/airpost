import React from 'react';
import {
  FocusContainer,
  ListensToFluxStore,
  ListDataSource,
  BindGlobalCommands,
} from 'mailspring-component-kit';
import { Button } from '../../components/kit/button';
import { IconButton } from '../../components/kit/icon-button';
import { Store, ContactsPerspective } from './store';
import {
  localized,
  DestroyContactTask,
  Actions,
  Contact,
  ChangeContactGroupMembershipTask,
} from 'mailspring-exports';
import { showGPeopleReadonlyNotice } from './google-support';
import { exportContactsToFile } from './files';

interface ContactDetailToolbarProps {
  editing: string | 'new' | false;
  listSource: ListDataSource;
  perspective: ContactsPerspective;
  focusedId?: string;
}

class ContactDetailToolbarWithData extends React.Component<ContactDetailToolbarProps> {
  constructor(props: ContactDetailToolbarProps) {
    super(props);
  }

  _onRemoveFromSource = () => {
    if (this.props.perspective.type !== 'group') {
      throw new Error('Remove from source but perspective is not a group');
      return;
    }
    if (showGPeopleReadonlyNotice(this.props.perspective.accountId)) {
      return;
    }

    const groupId = this.props.perspective.groupId;
    const group = Store.groups().find((g) => g.id === groupId);

    Actions.queueTask(
      ChangeContactGroupMembershipTask.forMoving({
        contacts: this.actionSet(),
        group: group,
        direction: 'remove',
      })
    );
  };

  _onEdit = () => {
    const contacts = this.actionSet();
    const contact = contacts[0];
    if (!contact || showGPeopleReadonlyNotice(contact.accountId)) {
      return;
    }
    Store.setEditing(contact.id);
  };

  _onDelete = () => {
    const contacts = this.actionSet();
    if (contacts.some((c) => c.source === 'gpeople' && showGPeopleReadonlyNotice(c.accountId))) {
      return;
    }
    Actions.queueTask(
      DestroyContactTask.forRemoving({
        contacts: this.actionSet(),
      })
    );
  };

  actionSet() {
    const { listSource, focusedId } = this.props;
    const focused = focusedId && listSource.getById(focusedId);
    const models = focused ? [focused] : listSource.selection.items();
    return models as any as Contact[];
  }

  render() {
    const { perspective, editing } = this.props;
    const actionSet = this.actionSet();
    const editable = actionSet.length === 1 && actionSet[0].source !== 'mail';

    if (editing) {
      return <span />;
    }

    const commands = {};
    if (perspective.type === 'group' && actionSet.length > 0) {
      commands['core:remove-from-view'] = this._onRemoveFromSource;
    }
    if (actionSet.length > 0) {
      commands['core:delete-item'] = this._onDelete;
      commands['contacts:export-vcf-selected'] = () => exportContactsToFile(actionSet);
    }
    if (editable) {
      commands['core:edit-item'] = this._onEdit;
    }

    return (
      <BindGlobalCommands key={Object.keys(commands).join(',')} commands={commands}>
        <div style={{ display: 'flex', order: 1000, marginRight: 10 }}>
          {perspective.type === 'group' && (
            <Button
              variant="toolbar"
              tabIndex={-1}
              disabled={actionSet.length === 0}
              onClick={this._onRemoveFromSource}
            >
              {localized('Remove from Group')}
            </Button>
          )}
          <IconButton
            variant="toolbar"
            tabIndex={-1}
            icon="toolbar-trash.png"
            label={localized('Delete')}
            disabled={actionSet.length === 0}
            onClick={this._onDelete}
          />
          <IconButton
            variant="toolbar"
            tabIndex={-1}
            icon="toolbar-export-contact.png"
            label={localized('Export vCard')}
            disabled={actionSet.length === 0}
            onClick={() => exportContactsToFile(actionSet)}
          />
          <Button variant="toolbar" tabIndex={-1} disabled={!editable} onClick={this._onEdit}>
            {localized('Edit')}
          </Button>
        </div>
      </BindGlobalCommands>
    );
  }
}

export const ContactDetailToolbar: React.FunctionComponent<ContactDetailToolbarProps> =
  ListensToFluxStore(
    ({ listSource, editing, perspective }) => (
      <FocusContainer collection="contact">
        <ContactDetailToolbarWithData
          listSource={listSource}
          editing={editing}
          perspective={perspective}
        />
      </FocusContainer>
    ),
    {
      stores: [Store],
      getStateFromStores: () => ({
        editing: Store.editing(),
        listSource: Store.listSource(),
        perspective: Store.perspective(),
      }),
    }
  );

ContactDetailToolbar.displayName = 'ContactDetailToolbar';
