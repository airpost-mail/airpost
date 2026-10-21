import { Account, AccountStore } from 'mailspring-exports';
import * as EngineAccounts from '../../engine/client/accounts';

// The servers for a new account come from the engine (`account.discover`: the provider
// tables, MX records and Thunderbird autoconfig, crates/tern-providers). What the user typed
// stays ahead of them.
export async function expandAccountWithCommonSettings(account: Account) {
  const { account: found } = await EngineAccounts.discover(account.emailAddress, account.provider);
  const populated = account.clone();
  const defaults = { ...(found.settings || {}) };
  // The password the basic form asked for opens SMTP too, unless it was given apart.
  if (!populated.settings.smtp_password && populated.settings.imap_password) {
    defaults.smtp_password = populated.settings.imap_password;
  }
  populated.settings = Object.assign(defaults, populated.settings);
  return applyContainerFolderDefault(populated);
}

// Proton accounts pin container_folder to "Folders" through the provider table; the others
// get the user's default.
function applyContainerFolderDefault(populated: Account) {
  const containerFolderDefault = AccountStore.containerFolderDefaultGetter();
  if (
    containerFolderDefault !== 'Mailspring' &&
    (populated.settings.container_folder === '' ||
      populated.settings.container_folder === undefined)
  ) {
    populated.settings.container_folder = containerFolderDefault;
  }
  return populated;
}

function tidy(account: Account) {
  if (account.settings.imap_host) {
    account.settings.imap_host = account.settings.imap_host.trim();
  }
  if (account.settings.smtp_host) {
    account.settings.smtp_host = account.settings.smtp_host.trim();
  }
  if (account.settings.imap_port) {
    account.settings.imap_port /= 1;
  }
  if (account.settings.smtp_port) {
    account.settings.smtp_port /= 1;
  }
  if (account.label && account.label.includes('@')) {
    account.label = account.emailAddress;
  }
}

// The engine tests the account and, when it passes, stores and starts it: a new account
// gets its id from the engine, a reconnected one (already in AccountStore) keeps its own.
// Returns the account without its secrets, which only the engine keeps.
export async function finalizeAndValidateAccount(account: Account) {
  tidy(account);
  const json = account.toJSON();
  const existing = account.id && AccountStore.accountForId(account.id);
  const added = existing
    ? await EngineAccounts.reconnectAccount(json)
    : await EngineAccounts.addAccount(json);
  account.id = added.accountId;
  for (const key of EngineAccounts.SECRET_SETTINGS) {
    delete account.settings[key];
  }
  account.authedAt = new Date();
  return account;
}

// The account `account.oauth.complete` added, as AccountStore keeps it.
export function accountFromSignIn(json: Record<string, any>) {
  const account = new Account({}).fromJSON(json);
  account.authedAt = new Date();
  return account;
}
