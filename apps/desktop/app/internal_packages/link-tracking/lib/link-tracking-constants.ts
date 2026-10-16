import plugin from '../package.json';

export const PLUGIN_NAME = plugin.title;
export const PLUGIN_ID = plugin.name;

// There is no tracking server yet. Until one is configured the composer leaves outgoing mail
// untouched and the toolbar toggle is disabled.
export const PLUGIN_URL: string | null = null;

export function isTrackingServerAvailable() {
  return PLUGIN_URL !== null;
}
