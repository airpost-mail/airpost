// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { localized } from '../../intl/localize';
import { configCredentialsKey, KeyManager, KeyManagerConfig, KeySet } from './key-manager';
import { MailspringKeychainError, readMailspringKeySet } from './mailspring-keychain';
import {
  ImportCancelledError,
  ImportResult,
  mailspringAccounts,
  newMailspringAccounts,
  readMailspringSettings,
} from './mailspring-import';

/*
The dialogs and the keychain step of Import from Mailspring (mailspring-import.ts has the files).
The keychain step reads the passwords Mailspring saved with Mailspring's key, read-only
(mailspring-keychain.ts), and stores them with Tern's. When that fails the user chooses between
cancelling and importing without them; the accounts then ask to be signed in again.
*/

// Main-process dialogs, which specs replace.
export const importDialogs = {
  showMessageBoxSync(options: Electron.MessageBoxSyncOptions): number {
    const { BrowserWindow, dialog } = require('electron');
    const parent = BrowserWindow.getFocusedWindow();
    return parent ? dialog.showMessageBoxSync(parent, options) : dialog.showMessageBoxSync(options);
  },
};

function message(type: 'info' | 'warning' | 'error', text: string, detail?: string) {
  importDialogs.showMessageBoxSync({ type, buttons: [localized('OK')], message: text, detail });
}

function keychainHint(err: Error) {
  const retryable = err instanceof MailspringKeychainError && err.retryable;
  let hint = retryable
    ? localized(
        'Cancel and try again later, allowing access if your keychain asks, or go on without them and sign in to each account again.'
      )
    : localized('Go on without them and sign in to each account again, or cancel.');
  if (retryable && process.platform === 'linux') {
    hint += localized(
      ' On Linux, reading them needs your desktop keyring to be running and unlocked, and the secret-tool command (package libsecret-tools) or kwallet-query.'
    );
  }
  return hint;
}

// True when the user goes on without the passwords Mailspring saved.
export function continueWithoutPasswords(err: Error) {
  const choice = importDialogs.showMessageBoxSync({
    type: 'warning',
    buttons: [localized('Cancel'), localized('Import Without Passwords')],
    defaultId: 0,
    cancelId: 0,
    message: localized('Tern could not bring over the passwords Mailspring saved'),
    detail: `${err.message}\n\n${keychainHint(err)}`,
  });
  return choice === 1;
}

// A KeyManager over a plain object, to encrypt a keyset with Tern's key outside Config.
function detachedKeyManager(holder: { [key: string]: any }) {
  const config: KeyManagerConfig = {
    get: (key) => holder[key],
    set: (key, value) => {
      holder[key] = value;
    },
  };
  return new KeyManager({ config });
}

/**
 * The keychain step of a whole-profile import: Mailspring's stored `credentials`, decrypted
 * with Mailspring's key and encrypted with Tern's, as config.json stores them. Null when there
 * are none or the user goes on without them; throws ImportCancelledError when the user cancels.
 */
export async function mailspringCredentialsForTern(stored: unknown, from: string) {
  try {
    const keys = await readMailspringKeySet(stored, from);
    if (!Object.keys(keys).length) return null;
    const holder: { [key: string]: any } = {};
    await detachedKeyManager(holder)._writeKeyHash(keys as KeySet);
    return holder[configCredentialsKey];
  } catch (err) {
    console.warn(`Tern could not bring over the passwords saved by Mailspring: ${err.message}`);
    if (continueWithoutPasswords(err)) return null;
    throw new ImportCancelledError();
  }
}

// Before the restart that imports a whole profile. True to go ahead.
export function confirmProfileImport(from: string) {
  const choice = importDialogs.showMessageBoxSync({
    type: 'question',
    buttons: [localized('Cancel'), localized('Import and Restart')],
    defaultId: 1,
    cancelId: 0,
    message: localized('Import from Mailspring?'),
    detail: localized(
      'Tern copies the accounts, saved passwords, settings, signatures, templates, mail rules, plugins and local drafts from %@, then restarts. The settings you made in Tern are replaced. Your mail is downloaded again.\n\nMailspring is only read, never changed. Your keychain may ask to let Tern read the passwords Mailspring saved.',
      from
    ),
  });
  return choice === 1;
}

// After a launch that imported a whole profile.
export function reportProfileImport(outcome: { result?: ImportResult; error?: Error }) {
  const { result, error } = outcome;
  if (error instanceof ImportCancelledError) return;
  if (error) {
    message(
      'error',
      localized('Tern could not import from Mailspring'),
      localized(
        '%@\n\nNothing in the Mailspring profile was changed. Tern starts with no accounts; you can import again from the welcome screen.',
        error.message
      )
    );
    return;
  }
  let detail = localized(
    'Your mail downloads again in the background. Local drafts, snoozes, reminders and scheduled sends come back as each account syncs. Mailspring was not changed.'
  );
  if (result.passwordsLeftBehind) {
    detail += `\n\n${localized(
      'The passwords Mailspring saved were not brought over, so each account asks you to sign in again.'
    )}`;
  }
  const title =
    result.accounts === 1
      ? localized('Imported 1 account from Mailspring')
      : localized('Imported %@ accounts from Mailspring', `${result.accounts}`);
  message('info', title, detail);
}

export type AddAccountsOutcome = 'added' | 'none' | 'cancelled' | 'failed';

/**
 * Import from Mailspring when Tern already has accounts: adds the Mailspring accounts Tern
 * doesn't have, with the passwords Mailspring saved, and changes nothing else (settings,
 * signatures, local drafts and the database stay Tern's).
 */
export async function addMailspringAccounts({
  from,
  config,
  keyManager,
}: {
  from: string;
  config: KeyManagerConfig;
  keyManager: KeyManager;
}): Promise<AddAccountsOutcome> {
  let settings;
  try {
    settings = readMailspringSettings(from);
  } catch (err) {
    message('error', localized('Tern could not read the Mailspring profile'), err.message);
    return 'failed';
  }
  const existing = config.get('accounts') || [];
  const adding = newMailspringAccounts(mailspringAccounts(settings), existing);
  if (!adding.length) {
    message(
      'info',
      localized('Nothing to import'),
      localized('Every account in Mailspring is already in Tern.')
    );
    return 'none';
  }

  const choice = importDialogs.showMessageBoxSync({
    type: 'question',
    buttons: [localized('Cancel'), localized('Add Accounts')],
    defaultId: 1,
    cancelId: 0,
    message: localized('Add accounts from Mailspring?'),
    detail: localized(
      '%@\n\nTern adds these accounts with the passwords Mailspring saved. Your accounts and settings in Tern stay as they are: settings, signatures and local drafts come over only when Tern has no accounts yet.\n\nMailspring is only read, never changed. Your keychain may ask to let Tern read the passwords Mailspring saved.',
      adding.map((a) => a.emailAddress).join('\n')
    ),
  });
  if (choice !== 1) return 'cancelled';

  let keys: KeySet = {};
  let passwordsLeftBehind = false;
  try {
    keys = (await readMailspringKeySet(settings.credentials, from)) as KeySet;
  } catch (err) {
    console.warn(`Tern could not bring over the passwords saved by Mailspring: ${err.message}`);
    if (!continueWithoutPasswords(err)) return 'cancelled';
    passwordsLeftBehind = true;
  }

  const added = [];
  for (const account of adding) {
    const withSecrets = await keyManager.insertAccountSecrets(account, keys);
    added.push(await keyManager.extractAndStoreAccountSecrets(withSecrets));
  }
  // AccountStore in every window reloads when the version changes; the engine supervisor adds
  // the accounts to the engine when `accounts` does.
  config.set('accounts', [...existing, ...added]);
  config.set('accountsVersion', (Number(config.get('accountsVersion')) || 0) + 1);

  let detail = localized('They sync in the background. Mailspring was not changed.');
  if (passwordsLeftBehind) {
    detail += `\n\n${localized('Sign in to each of them again: their passwords did not come over.')}`;
  }
  const title =
    added.length === 1
      ? localized('Added 1 account from Mailspring')
      : localized('Added %@ accounts from Mailspring', `${added.length}`);
  message('info', title, detail);
  return 'added';
}
