import { IdentityStore } from 'mailspring-exports';
import plugin from '../package.json';

export const PLUGIN_NAME = plugin.title;
export const PLUGIN_ID = plugin.name;
export const PLUGIN_URL = plugin.serverUrl[AppEnv.config.get('env')];

// The tracking server only knows about messages from a signed-in Mailspring ID, so without
// one the pixel or redirect would point at a server that can't attribute it.
export function isTrackingServerAvailable() {
  return !!IdentityStore.identity();
}
