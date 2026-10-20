// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Account setup on the window's own engine connection (docs/desktop-client-plan.md 4.5, stage
12): discovery, the OAuth sign-in, adding, testing and reconnecting an account, removing it,
and the secrets the engine keeps for the integrations. The engine derives new accounts' ids,
keeps every secret (none comes back) and starts the accounts it stores; config.json's
`accounts` follows with the account's public JSON.
*/
import { ipcRenderer } from 'electron';
import { localized } from '../shared/intl';
import EngineEvents from './engine-events';
import {
  AccountAddResult,
  AccountDiscoverResult,
  AccountSecrets,
  AccountTestResult,
  AccountUpdateResult,
  ErrorCode,
  Feature,
  Method,
  OAuthBeginParams,
  OAuthBeginResult,
  OAuthCompleteResult,
} from '../shared/engine-protocol';
import { errorFromEngineResult, stripSecretKeys } from '../shared/mailsync-process';
import { OAuthReceiverChannel, OAuthRedirect } from '../shared/supervisor-channels';

// How long a window waits for its connection's hello before giving up on a call.
const CONNECT_WAIT_MS = 20000;

// The port the browser's sign-in comes back to; registered with the providers' clients.
export const OAUTH_REDIRECT_PORT = 12141;

export const SECRET_SETTINGS = ['imap_password', 'smtp_password', 'refresh_token'] as const;

export function whenConnected(timeoutMs = CONNECT_WAIT_MS): Promise<void> {
  if (EngineEvents.features) return Promise.resolve();
  return new Promise<void>((resolve, reject) => {
    const onConnected = () => {
      clearTimeout(timer);
      resolve();
    };
    const timer = setTimeout(() => {
      EngineEvents.removeListener('connected', onConnected);
      reject(new Error(localized('The sync engine is not running. Restart Tern and try again.')));
    }, timeoutMs);
    EngineEvents.once('connected', onConnected);
  });
}

// Whether the connected engine takes the account methods on the socket.
export function accountSetupAvailable() {
  return EngineEvents.has(Feature.AccountSetup);
}

async function call<R>(method: string, params: unknown): Promise<R> {
  await whenConnected();
  if (!accountSetupAvailable()) {
    throw new Error(
      localized(`This sync engine can't set up accounts. Quit and reopen Tern to update it.`)
    );
  }
  try {
    return await EngineEvents.request<R>(method, params);
  } catch (err) {
    throw setupError(err);
  }
}

// An engine error as onboarding shows it: a failed test reads like the test's result.
export function setupError(err: any): Error {
  if (err && err.code === ErrorCode.AccountTestFailed && err.data) {
    return errorFromEngineResult(err.data as AccountTestResult, stripSecretKeys);
  }
  const error: any = err instanceof Error ? err : new Error(`${(err && err.message) || err}`);
  if (err && err.code === ErrorCode.OAuthFailed) {
    const data = (err.data || {}) as { key?: string; offline?: boolean };
    error.isNetworkError = data.offline === true;
    error.isUserError = ['no-mailbox', 'wrong-account'].includes(data.key);
    if (data.key === 'no-mailbox') {
      error.message = localized('There is no email mailbox associated with this account.');
    }
  }
  return error;
}

// An account's JSON without its secrets, and the secrets it had.
export function splitSecrets(json: Record<string, any>) {
  const account = JSON.parse(JSON.stringify(json));
  const secrets: AccountSecrets = {};
  const settings = account.settings || {};
  for (const key of SECRET_SETTINGS) {
    const value = settings[key];
    if (typeof value === 'string' && value) secrets[key] = value;
    delete settings[key];
  }
  // The engine keeps the client's view of the sync out of its registry.
  delete account.syncState;
  delete account.syncError;
  return { account, secrets };
}

export function discover(email: string, provider?: string) {
  return call<AccountDiscoverResult>(Method.AccountDiscover, { email, provider });
}

// Tests an account without adding it.
export async function testAccount(json: Record<string, any>) {
  const { account, secrets } = splitSecrets(json);
  const result = await call<AccountTestResult>(Method.AccountTest, { account, secrets });
  if (result.error) throw errorFromEngineResult(result, stripSecretKeys);
  return result;
}

// Tests the account, then stores and starts it. A new account gets its id here.
export function addAccount(json: Record<string, any>) {
  const { account, secrets } = splitSecrets(json);
  return call<AccountAddResult>(Method.AccountAdd, {
    account,
    secrets,
    test: true,
    identity: null,
  });
}

// Reconnect: the account keeps its id; the new settings go to account.update (which deletes
// the stored secrets when the servers changed) and the credentials to account.setSecrets.
// Tested first, so a failure changes nothing.
export async function reconnectAccount(json: Record<string, any>) {
  await testAccount(json);
  const { account, secrets } = splitSecrets(json);
  const patch: Record<string, any> = { ...account };
  delete patch.id;
  try {
    await call<AccountUpdateResult>(Method.AccountUpdate, { accountId: account.id, patch });
  } catch (err) {
    // Not registered (its secrets never reached the engine): added afresh, id kept.
    if ((err as any).code === ErrorCode.UnknownAccount) return addAccount(json);
    throw err;
  }
  if (!Object.keys(secrets).length) return { accountId: account.id, state: 'stopped' };
  return call<AccountAddResult>(Method.AccountSetSecrets, {
    accountId: account.id,
    secrets,
  });
}

export function updateAccount(accountId: string, patch: Record<string, any>) {
  return call<AccountUpdateResult>(Method.AccountUpdate, { accountId, patch });
}

export function reorderAccounts(ids: string[]) {
  return call(Method.AccountReorder, { ids });
}

// Stops the account, revokes a Google grant, deletes its rows and its secrets.
export function removeAccount(accountId: string) {
  return call(Method.AccountRemove, { accountId, revoke: true, deleteData: true });
}

export function setSecret(name: string, value: string) {
  return call(Method.SecretsSet, { name, value });
}

export function deleteSecret(name: string) {
  return call(Method.SecretsDelete, { name });
}

export function hasSecrets(names: string[]) {
  return call<{ [name: string]: boolean }>(Method.SecretsHas, { names });
}

// Starts a browser sign-in: the engine builds the authorization URL and keeps the PKCE
// verifier; the main process listens for the redirect. The caller opens `begun.authUrl`,
// waits for `redirect` and passes it to completeOAuth.
export async function beginOAuth(params: Omit<OAuthBeginParams, 'redirectPort'>) {
  const begun = await call<OAuthBeginResult>(Method.AccountOAuthBegin, {
    ...params,
    redirectPort: OAUTH_REDIRECT_PORT,
  });
  const redirect: Promise<OAuthRedirect> = ipcRenderer
    .invoke(OAuthReceiverChannel.Receive, { port: OAUTH_REDIRECT_PORT, state: begun.state })
    .then((r: OAuthRedirect) => {
      if (r.error) throw oauthRedirectError(r.error);
      return r;
    });
  return { begun, redirect };
}

// Exchanges the redirect's code: the engine tests, stores and starts the account.
export function completeOAuth(begun: OAuthBeginResult, redirect: OAuthRedirect) {
  return call<OAuthCompleteResult>(Method.AccountOAuthComplete, {
    sessionId: begun.sessionId,
    code: redirect.code,
    state: redirect.state,
  });
}

export function cancelOAuth(begun: OAuthBeginResult | null) {
  if (!begun) return Promise.resolve();
  return ipcRenderer.invoke(OAuthReceiverChannel.Cancel, begun.state);
}

function oauthRedirectError(reason: string) {
  const error: any = new Error(
    reason.startsWith('listen:')
      ? localized(
          `To receive the sign-in, Tern needs to listen on port %@ of this computer, and it couldn't (%@). Close other apps that may use it and try again.`,
          `${OAUTH_REDIRECT_PORT}`,
          reason.replace('listen: ', '')
        )
      : reason.startsWith('denied:')
        ? localized('The sign-in was cancelled or refused. Please try again.')
        : reason === 'timeout'
          ? localized('The sign-in took too long. Please try again.')
          : localized('The sign-in was cancelled.')
  );
  error.isUserError = true;
  error.cancelled = reason === 'cancelled' || reason === 'superseded';
  return error;
}
