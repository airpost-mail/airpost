import { CapabilityStore } from 'mailspring-exports';
import plugin from '../package.json';

export const PLUGIN_NAME = plugin.title;
export const PLUGIN_ID = plugin.name;

// Tracking needs a Tern server that offers it (docs/pro-features-backend.md). The engine adds
// the pixel and rewrites the links when it sends (docs/backend-plan.md 3.6); the toggle only
// writes the metadata, and is disabled while the capability store reports no such server.

export function isTrackingServerAvailable() {
  return CapabilityStore.isAvailable('read-receipts');
}

export function trackingUnavailableReason() {
  return CapabilityStore.unavailableMessage('read-receipts') || undefined;
}
