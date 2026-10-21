import React from 'react';
import ListensToFluxStore from '../../components/decorators/listens-to-flux-store';
import { localized } from '../../intl/localize';
import { Button } from '../../components/kit/button';

import { Store, ContactsPerspective } from './store';

interface FoundInMailEnabledBarProps {
  perspective: ContactsPerspective;
}

const CONFIG_KEY = 'core.contacts.findInMailDisabled';

class FoundInMailEnabledBarWithData extends React.Component<FoundInMailEnabledBarProps> {
  static displayName = 'FoundInMailEnabledBar';

  _onToggle = () => {
    const { perspective } = this.props;
    if (!perspective || perspective.type !== 'found-in-mail') {
      return false;
    }

    const accountId = perspective.accountId;
    let disabled = AppEnv.config.get(CONFIG_KEY);
    if (disabled.includes(accountId)) {
      disabled = disabled.filter((i: string) => i !== accountId);
    } else {
      disabled = [...disabled, accountId];
    }
    AppEnv.config.set(CONFIG_KEY, disabled);
    this.forceUpdate();
  };

  render() {
    const { perspective } = this.props;
    if (!perspective || perspective.type !== 'found-in-mail') {
      return false;
    }

    const disabled = AppEnv.config.get(CONFIG_KEY).includes(perspective.accountId);

    return (
      <div className="found-in-mail-enabled-bar">
        <div className="notice">
          {localized(
            `Contacts you've emailed appear here and Tern can suggest them when composing new messages.`
          )}
        </div>
        <Button variant="outline" size="sm" onClick={this._onToggle}>
          {disabled ? localized(`Enable`) : localized(`Disable`)}
        </Button>
      </div>
    );
  }
}

export const FoundInMailEnabledBar = ListensToFluxStore(FoundInMailEnabledBarWithData, {
  stores: [Store],
  getStateFromStores: () => ({
    perspective: Store.perspective(),
  }),
});
FoundInMailEnabledBar.displayName = 'FoundInMailEnabledBar';
