import React from 'react';
import { localized } from '../../../intl/localize';
import MailRulesStore from '../../../data/stores/mail-rules';
import * as Actions from '../../../data/actions';
import Notification from '../../../components/notification';

export default class DisabledMailRulesNotification extends React.Component<
  Record<string, unknown>,
  { disabledRules: any[] }
> {
  static displayName = 'DisabledMailRulesNotification';

  unlisten?: () => void;

  constructor(props: Record<string, unknown>) {
    super(props);
    this.state = this.getStateFromStores();
  }

  componentDidMount() {
    this.unlisten = MailRulesStore.listen(() => this.setState(this.getStateFromStores()));
  }

  componentWillUnmount() {
    if (this.unlisten) this.unlisten();
  }

  getStateFromStores() {
    return {
      disabledRules: MailRulesStore.disabledRules(),
    };
  }

  _onOpenMailRulesPreferences = () => {
    Actions.switchPreferencesTab('Mail Rules', {
      accountId: this.state.disabledRules[0].accountId,
    });
    Actions.openPreferences();
  };

  render() {
    if (this.state.disabledRules.length === 0) {
      return <span />;
    }
    return (
      <Notification
        priority="2"
        title={localized('One or more of your mail rules have been disabled.')}
        icon="volstead-defaultclient.png"
        isError
        actions={[
          {
            label: localized('View Mail Rules'),
            fn: this._onOpenMailRulesPreferences,
          },
        ]}
      />
    );
  }
}
