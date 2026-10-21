import React from 'react';
import { localized } from '../../../intl/localize';
import {
  SettingsPage,
  SettingsSection,
  ConfigSwitchRow,
  ConfigSelectRow,
  SettingsRow,
  NeedsServerPill,
} from '../../../components/settings';

export default function PreferencesNotifications() {
  const enabled = !!AppEnv.config.get('core.notifications.enabled');
  return (
    <SettingsPage
      title={localized('Notifications')}
      description={localized('When Tern gets your attention.')}
    >
      <SettingsSection title={localized('New mail')}>
        <ConfigSwitchRow
          keyPath="core.notifications.enabled"
          label={localized('Notify me about new mail')}
        />
        <ConfigSwitchRow
          keyPath="core.notifications.sounds"
          label={localized('Play a sound')}
          disabled={!enabled}
        />
      </SettingsSection>

      <SettingsSection title={localized('App icon')}>
        <ConfigSelectRow
          keyPath="core.notifications.countBadge"
          label={localized('Badge')}
          options={[
            ['unread', localized('Unread count')],
            ['total', localized('Total count')],
            ['hide', localized('Off')],
          ]}
        />
        <ConfigSwitchRow
          keyPath="core.notifications.countBadgeAllAccounts"
          label={localized('Count unread mail in every account')}
          description={localized("Off: only the folder you're looking at.")}
        />
      </SettingsSection>

      <SettingsSection title={localized('Snooze and tracking')}>
        <ConfigSwitchRow
          keyPath="core.notifications.unsnoozeToTop"
          label={localized('Move snoozed mail to the top of the inbox when it comes back')}
        />
        <SettingsRow
          label={localized('Tell me again when someone reopens my mail or clicks a link')}
          description={localized('Read receipts and link tracking go through a Tern server.')}
          disabled
        >
          <NeedsServerPill />
        </SettingsRow>
      </SettingsSection>
    </SettingsPage>
  );
}

PreferencesNotifications.displayName = 'PreferencesNotifications';
