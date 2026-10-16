import { CapabilityStore } from 'mailspring-exports';
import plugin from '../package.json';

export const PLUGIN_NAME = plugin.title;
export const PLUGIN_ID = plugin.name;

// Tracking needs a Tern server (docs/pro-features-backend.md). Until the capability store
// reports one, the composer leaves outgoing mail untouched and the toolbar toggle is disabled.
// B1-C replaces PLUGIN_URL with URLs built from the connected server.
export const PLUGIN_URL: string | null = null;

export function isTrackingServerAvailable() {
  return CapabilityStore.isAvailable('link-tracking') && PLUGIN_URL !== null;
}

export function trackingUnavailableReason() {
  return CapabilityStore.unavailableMessage('link-tracking') || undefined;
}
