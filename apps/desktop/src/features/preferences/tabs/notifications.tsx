import React from 'react';
import { pathToFileURL } from 'url';
import { localized } from '../../../intl/localize';
import { macSystemSoundNames, macSystemSoundPath } from '../../../platform/system-sounds';
import { playSoundURL } from '../../../packages/sound-registry';
import {
  SettingsPage,
  SettingsSection,
  ConfigSwitchRow,
  ConfigSelectRow,
  SettingsRow,
  NeedsServerPill,
} from '../../../components/settings';

// Mail.app's New Mail and the sounds in /System/Library/Sounds, macOS only; elsewhere new mail
// plays the OS sound theme's or the bundled sound.
const MacSounds = macSystemSoundNames();

function previewMacSound(name: unknown) {
  const file = macSystemSoundPath(name);
  if (!file) return;
  playSoundURL(pathToFileURL(file).href);
}

export default function PreferencesNotifications() {
  const enabled = !!AppEnv.config.get('core.notifications.enabled');
  const sounds = !!AppEnv.config.get('core.notifications.sounds');
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
        {MacSounds.length > 0 && (
          <ConfigSelectRow
            keyPath="core.notifications.sound"
            label={localized('Sound')}
            options={MacSounds.map((name) => [name, name])}
            disabled={!enabled || !sounds}
            onChange={previewMacSound}
          />
        )}
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
