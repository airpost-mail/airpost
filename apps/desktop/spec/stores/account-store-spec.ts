// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { _AccountStore } from '../../src/flux/stores/account-store';
import { CONFIG_ACCOUNTS_TO_REMOVE_KEY } from '../../src/shared/config-keys';
import { EngineEvents } from '../../src/engine/engine-events';
import { Feature, Method } from '../../src/shared/engine-protocol';

const flush = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

const accountJSON = (id: string, extra: Record<string, any> = {}): Record<string, any> => ({
  id,
  __cls: 'Account',
  provider: 'imap',
  emailAddress: `${id}@example.com`,
  label: `${id} label`,
  syncState: 'ok',
  settings: { imap_host: 'imap.example.com', imap_port: 993 },
  ...extra,
});

describe('AccountStore with an engine that keeps the accounts', function () {
  beforeEach(function () {
    spyOn(AppEnv, 'isMainWindow').andReturn(true);
    AppEnv.config.set('accounts', [accountJSON('a1', { syncState: 'invalid' })]);
    this.engine = new EngineEvents();
    this.requests = [];
    this.listed = [];
    this.request = (method: string, params: unknown) => {
      this.requests.push({ method, params });
      return Promise.resolve({ accounts: this.listed.map((account) => ({ account })) });
    };
  });

  const ids = (store: _AccountStore) => store.accounts().map((a) => a.id);
  const saved = () => (AppEnv.config.get('accounts') || []).map((a) => a.id);

  it('keeps to config.json without Feature.Accounts', function () {
    waitsForPromise(async () => {
      const store = new _AccountStore(this.engine);
      this.engine.connected([Feature.Counts], this.request);
      this.engine.notification(Method.AccountChanged, {
        accountId: 'b1',
        account: accountJSON('b1'),
      });
      await flush();
      expect(this.requests).toEqual([]);
      expect(ids(store)).toEqual(['a1']);
    });
  });

  it('lists the accounts another client added, without overwriting its own', function () {
    waitsForPromise(async () => {
      this.listed = [accountJSON('a1', { label: 'older copy' }), accountJSON('b1')];
      const store = new _AccountStore(this.engine);
      this.engine.connected([Feature.Accounts], this.request);
      await flush();

      expect(this.requests).toEqual([{ method: Method.AccountList, params: {} }]);
      expect(ids(store)).toEqual(['a1', 'b1']);
      expect(saved()).toEqual(['a1', 'b1']);
      expect(store.accountForId('a1').label).toBe('a1 label');
    });
  });

  it('applies account.changed but keeps its own sync state', function () {
    waitsForPromise(async () => {
      const store = new _AccountStore(this.engine);
      this.engine.connected([Feature.Accounts], this.request);
      await flush();
      let triggers = 0;
      store.listen(() => triggers++);

      const changed = accountJSON('a1', { label: 'Work', syncState: 'ok' });
      changed.settings = { imap_host: 'imap.example.com', imap_port: 993, container_folder: 'X' };
      this.engine.notification(Method.AccountChanged, { accountId: 'a1', account: changed });
      await flush();
      const a1 = store.accountForId('a1');
      expect(a1.label).toBe('Work');
      expect(a1.settings.container_folder).toBe('X');
      expect(a1.syncState).toBe('invalid');
      expect(triggers).toBe(1);

      // The same JSON again changes nothing.
      this.engine.notification(Method.AccountChanged, { accountId: 'a1', account: changed });
      await flush();
      expect(triggers).toBe(1);
    });
  });

  it('leaves out the accounts deleted in the app', function () {
    waitsForPromise(async () => {
      AppEnv.config.set(CONFIG_ACCOUNTS_TO_REMOVE_KEY, ['gone']);
      this.listed = [accountJSON('gone')];
      const store = new _AccountStore(this.engine);
      this.engine.connected([Feature.Accounts], this.request);
      this.engine.notification(Method.AccountChanged, {
        accountId: 'gone',
        account: accountJSON('gone'),
      });
      // Another account with an address the store already has.
      this.engine.notification(Method.AccountChanged, {
        accountId: 'dup',
        account: accountJSON('dup', { emailAddress: 'a1@example.com' }),
      });
      await flush();
      expect(ids(store)).toEqual(['a1']);
    });
  });
});
