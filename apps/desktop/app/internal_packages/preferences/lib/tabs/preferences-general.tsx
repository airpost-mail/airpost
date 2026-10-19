import React from 'react';
import { localized } from 'mailspring-exports';
import { SettingsPage, SettingsSection, ConfigSwitchRow } from 'mailspring-component-kit';
import { DefaultMailClientRow, LaunchOnStartRow } from './startup-rows';
import LanguageRow from './language-row';
import { ConfigLike, ConfigSchemaLike } from '../types';

/** On macOS Tern always outlives its window, so the setting only shows or hides the menu bar
 * icon; elsewhere the tray icon is what keeps Tern running after the window closes
 * (`MailspringWindow`'s close handler). */
export function BackgroundRow() {
  if (process.platform === 'darwin') {
    return (
      <ConfigSwitchRow
        keyPath="core.workspace.systemTray"
        label={localized('Show Tern in the menu bar')}
        description={localized(
          'Tern keeps running after you close the window, so scheduled sends go out and agents keep working.'
        )}
      />
    );
  }
  return (
    <ConfigSwitchRow
      keyPath="core.workspace.systemTray"
      label={localized('Keep Tern running in the system tray')}
      description={
        process.platform === 'linux'
          ? localized(
              'Closing the window leaves Tern running, so scheduled sends go out and agents keep working. The tray icon may need libappindicator; turning it off takes effect after a restart.'
            )
          : localized(
              'Closing the window leaves Tern running, so scheduled sends go out and agents keep working. Quit from the tray icon.'
            )
      }
    />
  );
}

export default function PreferencesGeneral(props: {
  config: ConfigLike;
  configSchema: ConfigSchemaLike;
}) {
  return (
    <SettingsPage title={localized('General')} description={localized('How Tern starts and runs.')}>
      <SettingsSection title={localized('Startup')}>
        <LaunchOnStartRow />
        <BackgroundRow />
      </SettingsSection>

      <SettingsSection title={localized('Default mail app')}>
        <DefaultMailClientRow />
      </SettingsSection>

      <SettingsSection title={localized('Language and time')}>
        <LanguageRow config={props.config} />
        <ConfigSwitchRow
          keyPath="core.workspace.use24HourClock"
          label={localized('Use 24-hour time')}
        />
      </SettingsSection>
    </SettingsPage>
  );
}

PreferencesGeneral.displayName = 'PreferencesGeneral';
