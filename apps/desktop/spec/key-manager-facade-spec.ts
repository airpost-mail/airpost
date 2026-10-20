// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { Account } from 'mailspring-exports';
import { KeyManagerFacade } from '../src/key-manager';
import { KEY_MANAGER_CHANNEL } from '../src/shared/supervisor-channels';

// The windows' KeyManager, answered by a stand-in for the main process: synthetic keys only.
describe('KeyManager in a window', function () {
  let km: KeyManagerFacade = null;
  let calls: any[][] = [];
  let keys: { [key: string]: string } = {};

  beforeEach(function () {
    calls = [];
    keys = { 'me@example.com-imap': 'pw', 'me@example.com-refresh-token': 'rt' };
    km = new KeyManagerFacade();
    km.invoke = async (channel: string, method: string, ...args: any[]) => {
      expect(channel).toBe(KEY_MANAGER_CHANNEL);
      calls.push([method, ...args]);
      switch (method) {
        case 'accountSecrets':
          return {
            value: {
              imap_password: keys[`${args[0]}-imap`],
              smtp_password: keys[`${args[0]}-smtp`],
              refresh_token: keys[`${args[0]}-refresh-token`],
            },
          };
        case 'getPassword':
          return { value: keys[args[0]] };
        case 'getKeyHash':
          return { value: { ...keys } };
        case 'replacePassword':
          return { error: 'Tern could not store your password securely.', noSentry: true };
        default:
          return { value: undefined };
      }
    };
  });

  const account = () =>
    new Account({
      id: 'acc1',
      emailAddress: 'me@example.com',
      settings: { imap_host: 'imap.example.com', imap_password: 'new-pw', refresh_token: 'new-rt' },
    });

  it('sends the secrets to main and keeps a copy without them', async function () {
    const clean = await km.extractAndStoreAccountSecrets(account());
    expect(calls).toEqual([
      [
        'storeAccountSecrets',
        {
          emailAddress: 'me@example.com',
          settings: { imap_password: 'new-pw', smtp_password: undefined, refresh_token: 'new-rt' },
        },
      ],
    ]);
    expect(clean.settings).toEqual({ imap_host: 'imap.example.com' });
    expect(clean instanceof Account).toBe(true);
  });

  it('reads one account’s secrets from main', async function () {
    const full = await km.insertAccountSecrets(account());
    expect(calls).toEqual([['accountSecrets', 'me@example.com']]);
    expect(full.settings.imap_password).toBe('pw');
    expect(full.settings.refresh_token).toBe('rt');
    expect(full.settings.imap_host).toBe('imap.example.com');
  });

  it('uses a keyset it was given without asking main', async function () {
    const full = await km.insertAccountSecrets(account(), { 'me@example.com-smtp': 'smtp' });
    expect(calls).toEqual([]);
    expect(full.settings.smtp_password).toBe('smtp');
    expect(full.settings.imap_password).toBe(undefined);
  });

  it('keeps the password methods', async function () {
    expect(await km.getPassword('me@example.com-imap')).toBe('pw');
    await km.deletePassword('integration-deepl-key');
    await km.deleteAccountSecrets(account());
    expect(await km._getKeyHash()).toEqual(keys);
    expect(calls.map((c) => c[0])).toEqual([
      'getPassword',
      'deletePassword',
      'deleteAccountSecrets',
      'getKeyHash',
    ]);
    expect(calls[2]).toEqual(['deleteAccountSecrets', 'me@example.com']);
  });

  it('rethrows what main failed with, marked when main already showed a dialog', async function () {
    let raised: any = null;
    try {
      await km.replacePassword('integration-deepl-key', 'synthetic');
    } catch (err) {
      raised = err;
    }
    expect(raised.message).toBe('Tern could not store your password securely.');
    expect(raised.noSentry).toBe(true);
  });
});
