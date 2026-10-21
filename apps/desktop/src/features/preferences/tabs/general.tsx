import React from 'react';
import { localized } from '../../../intl/localize';
import { SettingsPage, SettingsSection, ConfigSwitchRow } from '../../../components/settings';
import { DefaultMailClientRow, LaunchOnStartRow } from './startup-rows';
import LanguageRow from './language-row';
import { CommandLineRow } from './command-line-row';
import { EngineUnitRow, McpHttpRow, WebAccessRows } from './services-rows';
import { ConfigLike, ConfigSchemaLike } from '../types';

/** On macOS Tern always outlives its window, so the setting only shows or hides the menu bar
 * icon; elsewhere the tray icon is what keeps Tern running after the window closes
 * (`AppWindow`'s close handler). */
export function BackgroundRow() {
  if (process.platform === 'darwin') {
    return (
      <ConfigSwitchRow
        keyPath="core.workspace.systemTray"
        label={localized('Show Tern in the menu bar')}
        description={localized(
          'Tern keeps running after you close the window, so scheduled sends go out and agents keep working. To keep them going after you quit Tern, use Keep Tern running in the background below.'
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
              'Closing the window leaves Tern running, so scheduled sends go out and agents keep working. The tray icon may need libappindicator; turning it off takes effect after a restart. Keep Tern running in the background, below, does this without the tray icon.'
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

      <SettingsSection title={localized('Services on this computer')}>
        <EngineUnitRow />
        <WebAccessRows />
        <McpHttpRow />
        <CommandLineRow />
      </SettingsSection>
    </SettingsPage>
  );
}

PreferencesGeneral.displayName = 'PreferencesGeneral';
