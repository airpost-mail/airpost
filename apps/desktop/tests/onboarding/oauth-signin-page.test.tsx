import React from 'react';
import ReactDOM from 'react-dom';
import { ipcRenderer, shell } from 'electron';
import { AccountStore } from '../../src/data/stores/account';
import EngineEvents from '../../src/engine/client/events';
import { Feature } from '../../src/engine/protocol';
import AccountProviders from '../../src/features/onboarding/account-providers';
import OAuthSignInPage from '../../src/features/onboarding/pages/oauth-signin';

// The params of the page's account.oauth.begin, the redirect left pending.
async function beginParams(props: { accountId?: string } = {}) {
  const calls: any[] = [];
  EngineEvents.features = new Set([Feature.AccountSetup]);
  spyOn(EngineEvents, 'request').andCallFake((method: string, params: any) => {
    calls.push({ method, params });
    return Promise.resolve({
      sessionId: 's',
      authUrl: 'https://accounts.example/auth',
      redirectUri: 'http://127.0.0.1:12141',
      state: 'st',
    });
  });
  spyOn(ipcRenderer, 'invoke').andReturn(new Promise(() => {}));
  spyOn(shell, 'openExternal');
  const node = document.createElement('div');
  ReactDOM.render(
    <OAuthSignInPage
      provider="gmail"
      accountId={props.accountId}
      onSuccess={() => {}}
      onTryAgain={() => {}}
      providerConfig={AccountProviders[0]}
      serviceName="Google"
    />,
    node
  );
  for (let i = 0; i < 20 && !calls.length; i++) await Promise.resolve();
  ReactDOM.unmountComponentAtNode(node);
  EngineEvents.features = null;
  const begin = calls.find((c) => c.method === 'account.oauth.begin');
  expect(begin).toBeDefined();
  return begin.params;
}

describe('OAuthSignInPage', function oauthSignInPageSpecs() {
  it("gives a new account the user's default container folder", async () => {
    spyOn(AccountStore, 'containerFolderDefaultGetter').andReturn('Mail');
    const params = await beginParams();
    expect(params.provider).toEqual('gmail');
    expect(params.containerFolder).toEqual('Mail');
  });

  it("leaves the engine's default to the engine", async () => {
    spyOn(AccountStore, 'containerFolderDefaultGetter').andReturn('Tern');
    expect((await beginParams()).containerFolder).toBeUndefined();
  });

  it('passes a Mailspring container folder on, since the engine defaults to Tern', async () => {
    spyOn(AccountStore, 'containerFolderDefaultGetter').andReturn('Mailspring');
    expect((await beginParams()).containerFolder).toEqual('Mailspring');
  });

  it("keeps a reconnected account's folder", async () => {
    spyOn(AccountStore, 'containerFolderDefaultGetter').andReturn('Mail');
    expect((await beginParams({ accountId: 'a1' })).containerFolder).toBeUndefined();
  });
});
