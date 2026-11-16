import React, { Component } from 'react';
import { localized } from '../../../intl/localize';
import { Account } from '../../../data/models/account';
import { RetinaImg } from '../../../components/retina-img';
import AccountProviders from '../account-providers';

class AccountOnboardingSuccess extends Component<{ account: Account }> {
  // eslint-disable-line
  static displayName = 'AccountOnboardingSuccess';

  render() {
    const { account } = this.props;
    // The account's provider was picked from AccountProviders.
    const providerConfig = AccountProviders.find(({ provider }) => provider === account.provider)!;

    return (
      <div className={`page account-setup AccountOnboardingSuccess`}>
        <div className="logo-container">
          <RetinaImg
            style={{ backgroundColor: providerConfig.color, borderRadius: 44 }}
            name={providerConfig.headerIcon}
            mode={RetinaImg.Mode.ContentPreserve}
            className="logo"
          />
        </div>
        <div>
          <h2>
            {localized(
              'Successfully connected to %@!',
              providerConfig.displayNameShort || providerConfig.displayName
            )}
          </h2>
          <h3>{localized('Adding your account to Airpost…')}</h3>
        </div>
      </div>
    );
  }
}

export default AccountOnboardingSuccess;
