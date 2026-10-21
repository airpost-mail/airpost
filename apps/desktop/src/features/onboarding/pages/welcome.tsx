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
        <div className="steps-container">
          <div>
            <img
              className="logo"
              src="tern-asset://onboarding/assets/tern-bird-on-dark-256.png"
              alt=""
              style={{ width: 128, height: 128, marginTop: 110 }}
            />
            <p className="hero-text" style={{ fontSize: 46, marginTop: 19 }}>
              {localized('Welcome to Tern')}
            </p>
          </div>
        </div>
        <div className="footer">
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
