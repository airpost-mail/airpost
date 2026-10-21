import React from 'react';

import { Account } from '../../../data/models/account';

import OAuthSignInPage, { reconnectingAccountId } from './oauth-signin';
import * as OnboardingActions from '../actions';
import AccountProviders from '../account-providers';

export default class AccountSettingsPageOutlook extends React.Component<{ account: Account }> {
  static displayName = 'AccountSettingsPageOutlook';

  onSuccess(account) {
    OnboardingActions.finishAndAddAccount(account);
  }

  render() {
    const providerConfig = AccountProviders.find((a) => a.provider === this.props.account.provider);
    const goBack = () => OnboardingActions.moveToPreviousPage();

    return (
      <OAuthSignInPage
        serviceName="Outlook"
        provider="outlook"
        accountId={reconnectingAccountId(this.props.account)}
        providerConfig={providerConfig}
        onSuccess={this.onSuccess}
        onTryAgain={goBack}
      />
    );
  }
}
