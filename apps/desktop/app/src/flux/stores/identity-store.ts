import MailspringStore from 'mailspring-store';

import * as Utils from '../models/utils';
import * as Actions from '../actions';
import KeyManager from '../../key-manager';
import { makeRequest } from '../mailspring-api-request';
import { Disposable } from 'event-kit';
import { debounce } from 'underscore';

const PASSWORD_NAME = 'Mailspring Account';

export interface IIdentity {
  id: string;
  token: string;
  firstName: string;
  lastName: string;
  emailAddress: string;
}

class _IdentityStore extends MailspringStore {
  _identity: IIdentity = null;
  _disp: Disposable;

  constructor() {
    super();

    if (AppEnv.isEmptyWindow()) {
      /*
      Hot windows don't receive any action-bridge-messages, which include DB updates.
      Since the hot window loads first, it may have a stale verison of the Identity.
      */
      AppEnv.onWindowPropsReceived(() => {
        this._onIdentityChanged();
      });
      return;
    }

    AppEnv.config.onDidChange('identity', this._onIdentityChanged);
    this._onIdentityChanged();

    this.listenTo(Actions.logoutMailspringIdentity, this._onLogoutMailspringIdentity);
  }

  deactivate() {
    if (this._disp) this._disp.dispose();
    this.stopListeningToAll();
  }

  identity() {
    if (!this._identity || !this._identity.id) return null;
    return Utils.deepClone(this._identity);
  }

  identityId() {
    if (!this._identity) {
      return null;
    }
    return this._identity.id;
  }

  async saveIdentity(identity: IIdentity | null) {
    if (!identity) {
      this._identity = null;
      await KeyManager.deletePassword(PASSWORD_NAME);
      AppEnv.config.set('identity', null);
      return;
    }

    const { token, ...rest } = identity;

    // allow someone to call saveIdentity without the token,
    // and only save it if it's been changed (expensive call.)
    const oldToken = this._identity ? this._identity.token : null;
    const nextToken = token || oldToken;

    if (nextToken && nextToken !== oldToken) {
      // Note: We /must/ await this because calling config.set below
      // will try to retrieve the password via getPassword.
      // If this fails, the app may quit here.
      await KeyManager.replacePassword(PASSWORD_NAME, nextToken);
    }

    this._identity = identity;
    this._identity.token = nextToken;
    AppEnv.config.set('identity', rest);

    // Setting AppEnv.config will trigger our onDidChange handler,
    // no need to trigger here.
  }

  /**
   * When the identity changes in the database, update our local store
   * cache and set the token from the keychain.
   */
  _onIdentityChanged = async () => {
    const value = AppEnv.config.get('identity');
    this._identity = value
      ? { ...value, token: await KeyManager.getPassword(PASSWORD_NAME) }
      : null;

    if (this._identity && !this._identity.token) {
      console.warn(
        `IdentityStore: the token for Mailspring ID ${this._identity.emailAddress} is missing from the keychain; continuing without an identity.`
      );
      this._identity = null;
    }

    this.trigger();
  };

  _onLogoutMailspringIdentity = async () => {
    // Do not touch the keychain or restart the app during specs.
    if (AppEnv.inSpecMode()) return;
    await this.saveIdentity(null);
    // We need to relaunch the app to clear the webview session
    // and prevent the webview from re signing in with the same MailspringID
    require('@electron/remote').app.relaunch();
    require('@electron/remote').app.quit();
  };

  fetchIdentitySoon = debounce(
    () => {
      this.fetchIdentity().catch((err) => {
        console.warn('IdentityStore: fetchIdentitySoon failed:', err);
      });
    },
    5000,
    true
  );

  async fetchIdentity() {
    if (!this._identity || !this._identity.token) {
      return null;
    }

    let json;
    try {
      json = await makeRequest({
        server: 'identity',
        path: '/api/me',
        method: 'GET',
      });
    } catch (err) {
      // Network failures here are routine (offline, timeouts, dropped connections)
      // and will be retried by the next poll - don't let them reject and become
      // unhandled promise rejections that get reported to Sentry.
      console.warn('IdentityStore.fetchIdentity failed:', err);
      return this._identity;
    }

    if (!json || !json.id) {
      AppEnv.reportError(new Error('/api/me returned invalid json'), json || {});
      return this._identity;
    }

    if (json.id !== this._identity.id) {
      console.log('Note: server returned a different identity object.');
    }

    const nextIdentity = Object.assign({}, this._identity, json);
    await this.saveIdentity(nextIdentity);
    return this._identity;
  }
}

export const IdentityStore = new _IdentityStore();
