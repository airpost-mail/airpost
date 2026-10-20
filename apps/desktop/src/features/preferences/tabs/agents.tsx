import React from 'react';
import { localized } from 'mailspring-exports';
import { SettingsPage } from 'mailspring-component-kit';
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
