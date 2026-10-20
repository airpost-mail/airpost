import React from 'react';
import { localized } from 'mailspring-exports';
import { RetinaImg } from 'mailspring-component-kit';
import * as OnboardingActions from '../actions';
import AccountProviders from '../account-providers';
import {
  ImportFromMailspringButton,
  useMailspringProfile,
} from '../../../components/mailspring-import';

// Shown only when there is a Mailspring profile on this computer.
function MailspringImportPrompt() {
  if (!useMailspringProfile()) return null;
  return (
    <div className="mailspring-import">
      <span>{localized('Used Mailspring on this computer?')}</span>
      <ImportFromMailspringButton variant="link" />
    </div>
  );
}

export default class AccountChoosePage extends React.Component<{ account?: object }> {
  static displayName = 'AccountChoosePage';

  _renderProviders() {
    return AccountProviders.map(({ icon, displayName, provider }) => (
      <div
        key={provider}
        className={`provider ${provider}`}
        onClick={() => OnboardingActions.chooseAccountProvider(provider)}
      >
        <div className="icon-container">
          <RetinaImg name={icon} mode={RetinaImg.Mode.ContentPreserve} className="icon" />
        </div>
        <span className="provider-name">{displayName}</span>
      </div>
    ));
  }

  render() {
    return (
      <div className="page account-choose">
        <h2>{localized('Connect an email account')}</h2>
        <div className="provider-list">{this._renderProviders()}</div>
        <MailspringImportPrompt />
      </div>
    );
  }
}
