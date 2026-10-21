import React from 'react';
import { Store, ContactsPerspective } from './store';
import { localized } from '../../intl/localize';
import * as Actions from '../../data/actions';
import { AccountStore } from '../../data/stores/account';
import * as Icons from './svg-icons';
import ListensToFluxStore from '../../components/decorators/listens-to-flux-store';
import BindGlobalCommands from '../../components/bind-global-commands';
import { showGPeopleReadonlyNotice } from './google-support';
import { Button } from '../../components/kit/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../components/kit/tooltip';

interface AddContactToolbarProps {
  editing: string | 'new' | false;
  perspective: ContactsPerspective;
  focusedId?: string;
}

class AddContactToolbarWithData extends React.Component<AddContactToolbarProps> {
  constructor(props: AddContactToolbarProps) {
    super(props);
  }

  onAdd = () => {
    const { perspective } = this.props;

    if (!('accountId' in perspective)) return;
    if (showGPeopleReadonlyNotice(perspective.accountId)) return;

    Actions.setFocus({ collection: 'contact', item: null });
    Store.setEditing('new');
  };

  render() {
    const { editing, perspective } = this.props;
    const enabled =
      'accountId' in perspective &&
      editing === false &&
      perspective.accountId &&
      perspective.type !== 'found-in-mail';
    const acct = 'accountId' in perspective && AccountStore.accountForId(perspective.accountId);
    const label = acct
      ? localized('New contact in %@', acct.label)
      : localized('Select an account to add a contact.');

    return (
      <div style={{ display: 'flex', order: 1000 }}>
        <BindGlobalCommands
          key={`${enabled}`}
          commands={enabled ? { 'core:add-item': this.onAdd } : {}}
        >
          <Tooltip>
            {/* The span is the trigger: a disabled button takes no pointer events, and the
                tooltip explains why it is disabled. */}
            <TooltipTrigger asChild>
              <span className="btn-new-contact-wrap">
                <Button
                  variant="toolbar"
                  disabled={!enabled}
                  className="btn-new-contact"
                  aria-label={label}
                  onClick={this.onAdd}
                >
                  <Icons.NewPerson />
                </Button>
              </span>
            </TooltipTrigger>
            <TooltipContent>{label}</TooltipContent>
          </Tooltip>
        </BindGlobalCommands>
      </div>
    );
  }
}

export const AddContactToolbar: React.FunctionComponent<AddContactToolbarProps> =
  ListensToFluxStore(
    ({ listSource, editing, perspective }) => (
      <AddContactToolbarWithData editing={editing} perspective={perspective} />
    ),
    {
      stores: [Store],
      getStateFromStores: () => ({
        editing: Store.editing(),
        perspective: Store.perspective(),
      }),
    }
  );

AddContactToolbar.displayName = 'AddContactToolbar';
