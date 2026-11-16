import React from 'react';
import { localized } from '../../../intl/localize';
import { SettingsPage } from '../../../components/settings';
import { McpSection } from './mcp/section';

export default function PreferencesMcp() {
  return (
    <SettingsPage
      title={localized('MCP')}
      description={localized(
        'Let AI apps on this computer work with your mail through Airpost’s MCP server.'
      )}
    >
      <McpSection />
    </SettingsPage>
  );
}

PreferencesMcp.displayName = 'PreferencesMcp';
