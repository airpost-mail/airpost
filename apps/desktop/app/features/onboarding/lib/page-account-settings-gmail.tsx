import React from 'react';

import { Account } from 'mailspring-exports';

import OAuthSignInPage, { reconnectingAccountId } from './oauth-signin-page';
import * as OnboardingActions from './onboarding-actions';
import AccountProviders from './account-providers';

export default class AccountSettingsPageGmail extends React.Component<{ account: Account }> {
  static displayName = 'AccountSettingsPageGmail';

  onSuccess(account) {
    OnboardingActions.finishAndAddAccount(account);
  }

  render() {
    const providerConfig = AccountProviders.find((a) => a.provider === this.props.account.provider);
    const goBack = () => OnboardingActions.moveToPreviousPage();

    return (
      <OAuthSignInPage
        serviceName="Google"
        provider="gmail"
        accountId={reconnectingAccountId(this.props.account)}
        providerConfig={providerConfig}
        onSuccess={this.onSuccess}
        onTryAgain={goBack}
      />
    );
  }
}
