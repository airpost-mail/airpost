import React from 'react';
import { ipcRenderer } from 'electron';
import * as Actions from '../../../data/actions';
import { localized } from '../../../intl/localize';
import { SettingsPage, SettingsSection, SettingsRow } from '../../../components/settings';
import { Button } from '../../../components/kit/button';

export default function PreferencesAdvanced() {
  return (
    <SettingsPage
      title={localized('Advanced')}
      description={localized(
        'Rarely needed. Change these if something looks wrong or support asks you to.'
      )}
    >
      <SettingsSection title={localized('Folders')}>
        <SettingsRow
          label={localized('Special folders')}
          description={localized(
            'Which folder each account uses for Sent, Drafts, Archive, Spam and Trash.'
          )}
        >
          <Button variant="outline" onClick={() => Actions.switchPreferencesTab('Folders')}>
            {localized('Choose')}
          </Button>
        </SettingsRow>
      </SettingsSection>

      <SettingsSection title={localized('Troubleshooting')}>
        <SettingsRow
          label={localized('Rebuild the mail database')}
          description={localized(
            'Downloads your mail again from every account. Your accounts and settings stay.'
          )}
        >
          <Button
            variant="outline"
            onClick={() => ipcRenderer.send('command', 'application:reset-database', {})}
          >
            {localized('Rebuild')}
          </Button>
        </SettingsRow>
      </SettingsSection>

      <SettingsSection title={localized('Reset')} danger>
        <SettingsRow
          label={localized('Remove all accounts and settings')}
          description={localized(
            "Tern starts over as if newly installed. Mail on your providers isn't touched."
          )}
        >
          <Button
            variant="destructive"
            onClick={() => ipcRenderer.send('command', 'application:reset-profile')}
          >
            {localized('Reset Tern…')}
          </Button>
        </SettingsRow>
      </SettingsSection>
    </SettingsPage>
  );
}

PreferencesAdvanced.displayName = 'PreferencesAdvanced';
