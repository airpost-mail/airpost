import { AccountStore } from '../../data/stores/account';
import { localized } from '../../intl/localize';
import { Dialogs } from '../../platform/app-api';

const CONTACTS_OAUTH_SCOPE_ADDED = new Date(1581867440474);

export const showGPeopleReadonlyNotice = (accountId: string) => {
  const acct = AccountStore.accountForId(accountId);
  if (
    acct &&
    acct.provider === 'gmail' &&
    (!acct.authedAt || acct.authedAt < CONTACTS_OAUTH_SCOPE_ADDED)
  ) {
    // The answer is the condition, not the dialog: callers stop at once and the notice stays up.
    Dialogs.showMessageBox(
      {
        message: localized(`Please re-authenticate with Google`),
        detail: localized(
          `To make changes to contacts in this account, you'll need to re-authorize Tern to access your data.\n\n` +
            `In Tern's main window, go to Preferences > Accounts, select this account, and click "Re-authenticate". ` +
            `You'll be prompted to give Tern additional permission to update and delete your contacts.`
        ),
      },
      'none'
    );
    return true;
  }
  return false;
};
