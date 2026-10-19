import React from 'react';
import { ipcRenderer } from 'electron';
import { Actions, localized } from 'mailspring-exports';
import { SettingsPage, SettingsSection, SettingsRow } from 'mailspring-component-kit';

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
          <button className="btn" onClick={() => Actions.switchPreferencesTab('Folders')}>
            {localized('Choose')}
          </button>
        </SettingsRow>
      </SettingsSection>

      <SettingsSection title={localized('Troubleshooting')}>
        <SettingsRow
          label={localized('Rebuild the mail database')}
          description={localized(
            'Downloads your mail again from every account. Your accounts and settings stay.'
          )}
        >
          <button
            className="btn"
            onClick={() => ipcRenderer.send('command', 'application:reset-database', {})}
          >
            {localized('Rebuild')}
          </button>
        </SettingsRow>
      </SettingsSection>

      <SettingsSection title={localized('Reset')} danger>
        <SettingsRow
          label={localized('Remove all accounts and settings')}
          description={localized(
            "Tern starts over as if newly installed. Mail on your providers isn't touched."
          )}
        >
          <button
            className="btn btn-danger"
            onClick={() => ipcRenderer.send('command', 'application:reset-profile')}
          >
            {localized('Reset Tern…')}
          </button>
        </SettingsRow>
      </SettingsSection>
    </SettingsPage>
  );
}

PreferencesAdvanced.displayName = 'PreferencesAdvanced';
