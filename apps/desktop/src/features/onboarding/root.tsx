import React from 'react';
import { CSSTransition, TransitionGroup } from 'react-transition-group';
import { Account } from '../../data/models/account';
import OnboardingStore from './store';
import PageTopBar from './pages/top-bar';

import WelcomePage from './pages/welcome';
import TutorialPage from './pages/tutorial';
import AccountChoosePage from './pages/account-choose';
import AccountSettingsPage from './pages/account-settings';
import AccountSettingsPageGmail from './pages/account-settings-gmail';
import AccountSettingsPageO365 from './pages/account-settings-o365';
import AccountSettingsPageIMAP from './pages/account-settings-imap';
import AccountOnboardingSuccess from './pages/account-onboarding-success';
import InitialPreferencesPage from './pages/initial-preferences';
import AccountSettingsPageOutlook from './pages/account-settings-outlook';

const PageComponents = {
  welcome: WelcomePage,
  tutorial: TutorialPage,
  'account-choose': AccountChoosePage,
  'account-settings': AccountSettingsPage,
  'account-settings-gmail': AccountSettingsPageGmail,
  'account-settings-o365': AccountSettingsPageO365,
  'account-settings-outlook': AccountSettingsPageOutlook,
  'account-settings-imap': AccountSettingsPageIMAP,
  'account-onboarding-success': AccountOnboardingSuccess,
  'initial-preferences': InitialPreferencesPage,
};

interface OnboardingRootState {
  pageDepth: number;
  page: string;
  account: Account;
}

export default class OnboardingRoot extends React.Component<
  Record<string, unknown>,
  OnboardingRootState
> {
  static displayName = 'OnboardingRoot';
  static containerRequired = false;

  unsubscribe?: () => void;

  constructor(props) {
    super(props);
    this.state = this._getStateFromStore();
  }

  componentDidMount() {
    this.unsubscribe = OnboardingStore.listen(this._onStateChanged, this);
    AppEnv.center();
    AppEnv.displayWindow();
  }

  componentWillUnmount() {
    if (this.unsubscribe) {
      this.unsubscribe();
    }
  }

  _getStateFromStore = () => {
    return {
      page: OnboardingStore.page(),
      pageDepth: OnboardingStore.pageDepth(),
      account: OnboardingStore.account(),
    };
  };

  _onStateChanged = () => {
    this.setState(this._getStateFromStore());
  };

  render() {
    const Component = PageComponents[this.state.page];
    if (!Component) {
      throw new Error(`Cannot find component for page: ${this.state.page}`);
    }

    return (
      <div className="page-frame">
        <PageTopBar
          pageDepth={this.state.pageDepth}
          allowMoveBack={!['initial-preferences', 'account-choose'].includes(this.state.page)}
        />
        <TransitionGroup component={null}>
          <CSSTransition key={this.state.page} classNames="alpha-fade" timeout={150}>
            <div className="page-container">
              <Component account={this.state.account} />
            </div>
          </CSSTransition>
        </TransitionGroup>
      </div>
    );
  }
}
