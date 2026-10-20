import { Account, AccountStore } from 'mailspring-exports';
import EngineEvents from '../../src/engine/events';
import { ErrorCode, Feature } from '../../src/shared/engine/protocol';
import {
  expandAccountWithCommonSettings,
  finalizeAndValidateAccount,
} from '../../src/features/onboarding/helpers';

function accountFor(emailAddress: string, provider = 'imap', settings = {}) {
  return new Account({ name: 'Test', emailAddress, provider, settings } as any);
}

// The engine's answers by method; each call is recorded.
function fakeEngine(answers: { [method: string]: (params: any) => any }) {
  const calls: Array<{ method: string; params: any }> = [];
  EngineEvents.features = new Set([Feature.AccountSetup]);
  spyOn(EngineEvents, 'request').andCallFake((method: string, params: any) => {
    calls.push({ method, params: JSON.parse(JSON.stringify(params)) });
    const answer = answers[method];
    if (!answer) return Promise.reject(new Error(`unexpected ${method}`));
    try {
      return Promise.resolve(answer(params));
    } catch (err) {
      return Promise.reject(err);
    }
  });
  return calls;
}

const DISCOVERED = {
  imap_host: 'imap.example.com',
  imap_port: 993,
  imap_security: 'SSL / TLS',
  imap_username: 'user@example.com',
  smtp_host: 'smtp.example.com',
  smtp_port: 465,
  smtp_security: 'SSL / TLS',
  smtp_username: 'user@example.com',
  container_folder: '',
};

describe('onboarding through the engine', function onboardingHelpersTests() {
  beforeEach(() => {
    spyOn(AccountStore, 'containerFolderDefaultGetter').andReturn('');
  });

  afterEach(() => {
    EngineEvents.features = null;
  });

  it('takes the servers from account.discover, keeping what the user typed', async () => {
    const calls = fakeEngine({
      'account.discover': () => ({ account: { settings: DISCOVERED }, source: 'fallback' }),
    });
    const account = await expandAccountWithCommonSettings(
      accountFor('user@example.com', 'imap', { imap_password: 'pw', imap_host: 'bridge.local' })
    );
    expect(calls[0]).toEqual({
      method: 'account.discover',
      params: { email: 'user@example.com', provider: 'imap' },
    });
    expect(account.settings.imap_host).toEqual('bridge.local');
    expect(account.settings.smtp_host).toEqual('smtp.example.com');
    expect(account.settings.smtp_password).toEqual('pw');
  });

  it('applies the container folder default when discovery has none', async () => {
    (AccountStore.containerFolderDefaultGetter as jasmine.Spy).andReturn('Mail');
    fakeEngine({
      'account.discover': () => ({ account: { settings: DISCOVERED }, source: 'fallback' }),
    });
    const account = await expandAccountWithCommonSettings(accountFor('user@example.com'));
    expect(account.settings.container_folder).toEqual('Mail');
  });

  it('adds a new account with its secrets apart and takes the engine id', async () => {
    const calls = fakeEngine({
      'account.add': () => ({ accountId: '6c210fa1', state: 'running', stored: true }),
    });
    const account = accountFor('user@example.com', 'imap', {
      ...DISCOVERED,
      imap_password: 'pw',
      smtp_password: 'pw',
    });
    const added = await finalizeAndValidateAccount(account);
    expect(calls.length).toEqual(1);
    const { method, params } = calls[0];
    expect(method).toEqual('account.add');
    expect(params.test).toBe(true);
    expect(params.secrets).toEqual({ imap_password: 'pw', smtp_password: 'pw' });
    expect(params.account.settings.imap_password).toBeUndefined();
    expect(params.account.id).toBeUndefined();
    expect(added.id).toEqual('6c210fa1');
    expect(added.settings.imap_password).toBeUndefined();
  });

  it('shows a failed test as the engine described it', async () => {
    fakeEngine({
      'account.add': () => {
        const err: any = new Error('Account test failed (imap): ErrorAuthentication');
        err.code = ErrorCode.AccountTestFailed;
        err.data = { error: 'ErrorAuthentication', error_service: 'imap', log: 'A1 NO' };
        throw err;
      },
    });
    let error = null;
    try {
      await finalizeAndValidateAccount(
        accountFor('user@example.com', 'imap', { ...DISCOVERED, imap_password: 'bad' })
      );
    } catch (err) {
      error = err;
    }
    expect(error.message).toContain('(IMAP)');
    expect(error.rawLog).toEqual('A1 NO');
  });

  it('reconnects an account it has: tested, updated, new secrets', async () => {
    const existing = accountFor('user@example.com', 'imap', DISCOVERED);
    existing.id = 'acct1';
    spyOn(AccountStore, 'accountForId').andCallFake((id) => (id === 'acct1' ? existing : null));
    const calls = fakeEngine({
      'account.test': () => ({ error: null, log: '' }),
      'account.update': () => ({ account: {}, secretsCleared: true }),
      'account.setSecrets': () => ({ accountId: 'acct1', state: 'running' }),
    });
    const changed = existing.clone();
    changed.settings.imap_host = 'imap2.example.com';
    changed.settings.imap_password = 'n3w';
    const added = await finalizeAndValidateAccount(changed);
    expect(calls.map((c) => c.method)).toEqual([
      'account.test',
      'account.update',
      'account.setSecrets',
    ]);
    expect(calls[1].params.accountId).toEqual('acct1');
    expect(calls[1].params.patch.settings.imap_host).toEqual('imap2.example.com');
    expect(calls[1].params.patch.settings.imap_password).toBeUndefined();
    expect(calls[2].params).toEqual({ accountId: 'acct1', secrets: { imap_password: 'n3w' } });
    expect(added.id).toEqual('acct1');
  });
});
