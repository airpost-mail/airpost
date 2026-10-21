import React from 'react';
import { localized } from '../../../intl/localize';
import { AccountStore } from '../../../data/stores/account';
import { imapUtf7 } from '../../../mail/imap-utf7';

interface SyncActivityProps {
  syncState: {
    [accountId: string]: {
      [folderId: string]: { bodyProgress: number; scanProgress: number; busy: boolean };
    };
  };
}
export class SyncActivity extends React.Component<SyncActivityProps> {
  static displayName = 'ExpandedSyncActivity';

  renderFolderProgress(
    folderPath: string,
    { bodyProgress, scanProgress, busy }: SyncActivityProps['syncState'][string][string]
  ) {
    let status = 'complete';
    let progressLabel = '';

    if (busy) {
      status = 'busy';
      if (scanProgress < 1) {
        progressLabel = localized(`Scanning`) + ` (${Math.round(scanProgress * 100)}%)`;
      } else if (bodyProgress < 1) {
        progressLabel = localized(`Caching mail`) + ` (${Math.round(bodyProgress * 100)}%)`;
      } else {
        progressLabel = localized(`Scanning`) + '...';
      }
    }

    const folderDisplayPath = imapUtf7.decode(folderPath);

    return (
      <div className={`folder-progress ${status}`} key={folderPath}>
        {folderDisplayPath} <span className="progress-label">{progressLabel}</span>
      </div>
    );
  }

  render() {
    const accountRows = Object.entries(this.props.syncState).map(
      ([accountId, accountSyncState]) => {
        const account = AccountStore.accountForId(accountId);
        if (!account) {
          return null;
        }

        let folderComponents: JSX.Element[] | JSX.Element = Object.entries(accountSyncState).map(
          ([folderPath, folderState]) => {
            return this.renderFolderProgress(folderPath, folderState);
          }
        );

        if (folderComponents.length === 0) {
          folderComponents = <div>Gathering folders...</div>;
        }

        return (
          <div className="account" key={accountId}>
            <h2>{account.emailAddress}</h2>
            {folderComponents}
          </div>
        );
      }
    );

    let accountComponents: React.ReactNode = accountRows;
    if (accountRows.length === 0) {
      accountComponents = (
        <div>
          <br />
          {localized('Looking for accounts...')}
        </div>
      );
    }

    return <div className="sync-activity">{accountComponents}</div>;
  }
}
