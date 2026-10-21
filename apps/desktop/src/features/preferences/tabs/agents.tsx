import React from 'react';
import { localized } from '../../../intl/localize';
import { SettingsPage } from '../../../components/settings';
import { McpSection } from './mcp/section';

export default function PreferencesAgents() {
  return (
    <SettingsPage
      title={localized('Agents')}
      description={localized('Let AI apps and scripts on this computer work with your mail.')}
    >
      <McpSection />
    </SettingsPage>
  );
}

PreferencesAgents.displayName = 'PreferencesAgents';
