import { shell } from 'electron';
import React from 'react';
import { localized, localizedReactFragment } from '../../../intl/localize';
import { Account } from '../../../data/models/account';
import { AccountStore } from '../../../data/stores/account';
import CopyButton from '../../../components/copy-button';
import { RetinaImg } from '../../../components/retina-img';

import FormErrorMessage from '../form-error-message';
import AccountProviders from '../account-providers';
import { Button } from '../../../components/kit/button';
import { accountFromSignIn } from '../helpers';
import * as EngineAccounts from '../../../engine/client/accounts';
import { OAuthBeginResult } from '../../../engine/protocol';

// The sign-in runs through the engine (`account.oauth.begin`/`complete`), which keeps the
// PKCE verifier, the `state` and the client secret; the main process receives the browser's
// redirect on 127.0.0.1 (src/main/ipc/oauth-receiver.ts).
interface OAuthSignInPageProps {
  provider: 'gmail' | 'office365' | 'outlook';
  // Reconnect: the account the sign-in renews.
  accountId?: string;
  sharedMailbox?: string;
  onSuccess: (account: Account) => void;
  onTryAgain: () => void;
  providerConfig: (typeof AccountProviders)[0];
  serviceName: string;
  children?: React.ReactNode;
}

type SignInError = Error & {
  cancelled?: boolean;
  isNetworkError?: boolean;
  isUserError?: boolean;
  rawLog?: string;
};

interface OAuthSignInPageState {
  authStage: string;
  authUrl: string | null;
  showAlternative: boolean;
  errorMessage?: string;
  errorLog?: string;
}

// The account being reconnected, when the onboarding window was opened for one.
export function reconnectingAccountId(account: Account) {
  return account && account.id && AccountStore.accountForId(account.id) ? account.id : undefined;
}

// The user's default container folder for a new account, as IMAP accounts get it
// (onboarding/helpers.ts). "Tern" is the engine's own default.
export function oauthContainerFolder() {
  const folder = AccountStore.containerFolderDefaultGetter();
  return folder && folder !== 'Tern' ? folder : undefined;
}

export default class OAuthSignInPage extends React.Component<
  OAuthSignInPageProps,
  OAuthSignInPageState
> {
  static displayName = 'OAuthSignInPage';

  _startTimer?: NodeJS.Timeout;
  _warnTimer?: NodeJS.Timeout;
  _mounted = false;
  _begun: OAuthBeginResult | null = null;

  state: OAuthSignInPageState = {
    authStage: 'initial',
    authUrl: null,
    showAlternative: false,
  };

  componentDidMount() {
    this._mounted = true;
    this._warnTimer = setTimeout(() => {
      if (!this._mounted) return;
      this.setState({ showAlternative: true });
    }, 1500);
    this._signIn();
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._startTimer) clearTimeout(this._startTimer);
    if (this._warnTimer) clearTimeout(this._warnTimer);
    EngineAccounts.cancelOAuth(this._begun);
  }

  async _signIn() {
    const mountedAt = Date.now();
    try {
      const { provider, accountId, sharedMailbox } = this.props;
      const { begun, redirect } = await EngineAccounts.beginOAuth({
        provider,
        accountId,
        sharedMailbox: sharedMailbox || undefined,
        containerFolder: accountId ? undefined : oauthContainerFolder(),
      });
      this._begun = begun;
      if (!this._mounted) {
        EngineAccounts.cancelOAuth(begun);
        return;
      }
      this.setState({ authUrl: begun.authUrl });
      // Show the "Sign in to ..." prompt for a moment before bouncing to the browser
      // (400 ms animation + 200 ms to read).
      this._startTimer = setTimeout(
        () => {
          if (this._mounted) shell.openExternal(begun.authUrl);
        },
        Math.max(0, 600 - (Date.now() - mountedAt))
      );
      const received = await redirect;
      if (!this._mounted) return;
      AppEnv.focus();
      this.setState({ authStage: 'buildingAccount' });
      const added = await EngineAccounts.completeOAuth(begun, received);
      if (!this._mounted) return;
      const account = accountFromSignIn(added.account);
      this.setState({ authStage: 'accountSuccess' });
      setTimeout(() => {
        if (!this._mounted) return;
        this.props.onSuccess(account);
      }, 400);
    } catch (e) {
      // EngineAccounts rejects with an Error carrying these flags (engine/client/accounts.ts).
      const err = e as SignInError;
      if (!this._mounted || err.cancelled) return;
      AppEnv.focus();
      this._onError(err);
    }
  }

  _onError(err: SignInError) {
    const isNetworkError = err.message?.includes('Failed to fetch') || err.isNetworkError;
    this.setState({
      authStage: 'error',
      errorMessage: isNetworkError
        ? localized(
            'A network error occurred. Please check your internet connection and try again.'
          )
        : err.message,
      errorLog: err.rawLog,
    });
    // Don't report network errors or user-configuration errors (e.g. account has no mailbox)
    // to Sentry — they are expected and shown directly to the user.
    if (!isNetworkError && !err.isUserError) {
      AppEnv.reportError(err);
    }
  }

  _renderHeader() {
    const authStage = this.state.authStage;
    if (authStage === 'initial') {
      return (
        <h2>
          {localizedReactFragment(
            'Sign in with %@ in %@ your browser.',
            this.props.serviceName,
            <br />
          )}
        </h2>
      );
    }
    if (authStage === 'buildingAccount') {
      return <h2>{localized('Connecting to %@…', this.props.serviceName)}</h2>;
    }
    if (authStage === 'accountSuccess') {
      return (
        <div>
          <h2>{localized('Successfully connected to %@!', this.props.serviceName)}</h2>
          <h3>{localized('Adding your account to Tern…')}</h3>
        </div>
      );
    }

    // Error
    const { note } = this.props.providerConfig;
    return (
      <div>
        <h2>{localized('Sorry, we had trouble logging you in')}</h2>
        <div className="error-region">
          <FormErrorMessage message={this.state.errorMessage} log={this.state.errorLog} />
          {note && <div className="message empty note">{note}</div>}
          <Button variant="outline" style={{ marginTop: 20 }} onClick={this.props.onTryAgain}>
            {localized('Try Again')}
          </Button>
        </div>
      </div>
    );
  }

  _renderAlternative() {
    let classnames = 'input hidden';
    if (this.state.showAlternative) {
      classnames += ' fadein';
    }

    return (
      <div className="alternative-auth">
        <div className={classnames}>
          <div style={{ marginTop: 40 }}>
            {localized(`Page didn't open? Paste this URL into your browser:`)}
          </div>
          <input type="url" className="url-copy-target" value={this.state.authUrl || ''} readOnly />
          <CopyButton className="copy-to-clipboard" text={this.state.authUrl || ''} />
        </div>
      </div>
    );
  }

  _renderNote() {
    if (this.state.authStage === 'error') return null;
    const { note } = this.props.providerConfig;
    if (!note) return null;
    return <div className="message empty note">{note}</div>;
  }

  render() {
    return (
      <div className={`page account-setup ${this.props.serviceName.toLowerCase()}`}>
        <div className="logo-container">
          <RetinaImg
            name={this.props.providerConfig.headerIcon}
            style={{ backgroundColor: this.props.providerConfig.color, borderRadius: 44 }}
            mode={RetinaImg.Mode.ContentPreserve}
            className="logo"
          />
        </div>
        {this._renderHeader()}
        {this._renderNote()}
        {this.state.authStage === 'initial' && this.props.children}
        {this._renderAlternative()}
      </div>
    );
  }
}
