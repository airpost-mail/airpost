// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The windows' KeyManager: the same methods as before, answered by the main process's
KeyManager (main/profile/key-manager.ts), which owns safeStorage, the keyset in config.json and the
Mailspring keychain migration. Exported to plugins as `KeyManager`.
*/
import { Account } from '../data/models/account';
import {
  KEY_MANAGER_CHANNEL,
  KeyManagerMethod,
  KeyManagerReply,
} from '../shared/engine/supervisor-channels';

interface KeySet {
  [key: string]: string;
}

type Invoke = (channel: string, ...args: any[]) => Promise<KeyManagerReply>;

const SECRET_SETTINGS = ['imap_password', 'smtp_password', 'refresh_token'];

export class KeyManagerFacade {
  // ipcRenderer.invoke; specs replace it.
  invoke: Invoke = (channel, ...args) => require('electron').ipcRenderer.invoke(channel, ...args);

  async _call<T = any>(method: KeyManagerMethod, ...args: any[]): Promise<T> {
    const reply = await this.invoke(KEY_MANAGER_CHANNEL, method, ...args);
    if (reply && reply.error !== undefined) {
      const err = new Error(reply.error);
      // The main process already showed the user a dialog and is quitting.
      if (reply.noSentry) (err as any).noSentry = true;
      throw err;
    }
    return reply ? reply.value : undefined;
  }

  async deleteAccountSecrets(account: Account) {
    await this._call('deleteAccountSecrets', account.emailAddress);
  }

  async extractAndStoreAccountSecrets(account: Account) {
    const settings: { [key: string]: any } = account.settings || {};
    await this._call('storeAccountSecrets', {
      emailAddress: account.emailAddress,
      settings: {
        imap_password: settings.imap_password,
        smtp_password: settings.smtp_password,
        refresh_token: settings.refresh_token,
      },
    });
    const next = account.clone();
    for (const key of SECRET_SETTINGS) delete next.settings[key];
    return next;
  }

  async insertAccountSecrets(account: Account, keys: KeySet = null) {
    const next = account.clone();
    const secrets = keys
      ? {
          imap_password: keys[`${account.emailAddress}-imap`],
          smtp_password: keys[`${account.emailAddress}-smtp`],
          refresh_token: keys[`${account.emailAddress}-refresh-token`],
        }
      : await this._call('accountSecrets', account.emailAddress);
    next.settings.imap_password = secrets.imap_password;
    next.settings.smtp_password = secrets.smtp_password;
    next.settings.refresh_token = secrets.refresh_token;
    return next;
  }

  async replacePassword(keyName: string, newVal: string) {
    await this._call('replacePassword', keyName, newVal);
  }

  async deletePassword(keyName: string) {
    await this._call('deletePassword', keyName);
  }

  async getPassword(keyName: string): Promise<string | undefined> {
    return this._call('getPassword', keyName);
  }

  async _getKeyHash(): Promise<KeySet> {
    return this._call('getKeyHash');
  }
}

export default new KeyManagerFacade();
