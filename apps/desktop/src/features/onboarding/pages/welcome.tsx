import { localized } from '../../../intl/localize';
import React from 'react';
import * as OnboardingActions from '../actions';
import { Button } from '../../../components/kit/button';

export default class WelcomePage extends React.Component {
  static displayName = 'WelcomePage';

  _onContinue = () => {
    OnboardingActions.moveToPage('tutorial');
  };

  render() {
    return (
      <div className="page welcome">
        <div className="welcome-inner">
          <img
            className="logo"
            src="airpost-asset://onboarding/assets/airpost-bird-on-dark-256.png"
            alt=""
          />
          <h1 className="hero-text">{localized('Welcome to Airpost')}</h1>
          <p className="sub-text">
            {localized('Fast, private email for every account you have.')}
          </p>
          <Button
            key="next"
            variant="outline"
            size="lg"
            className="btn-continue"
            onClick={this._onContinue}
          >
            {localized('Get Started')}
          </Button>
        </div>
      </div>
    );
  }
}
