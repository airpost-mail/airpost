import { pathToFileURL } from 'url';
import SoundRegistry from '../../packages/sound-registry';
import { SoundEvent, systemSoundPath } from '../../platform/system-sounds';

// Played where the OS has no fitting sound of its own (see platform/system-sounds.ts). They are
// synthesized by scripts/generate-sounds.sh in apps/desktop.
const BUNDLED: Record<SoundEvent, string> = {
  send: 'tern-asset://custom-sounds/sounds/send.ogg',
  'hit-send': 'tern-asset://custom-sounds/sounds/hit-send.ogg',
  'new-mail': 'tern-asset://custom-sounds/sounds/new-mail.ogg',
};

function soundURL(event: SoundEvent) {
  const macName = event === 'new-mail' ? AppEnv.config.get('core.notifications.sound') : undefined;
  const file = systemSoundPath(event, macName);
  return file ? pathToFileURL(file).href : BUNDLED[event];
}

export function activate() {
  for (const event of Object.keys(BUNDLED) as SoundEvent[]) {
    SoundRegistry.register(event, soundURL(event));
  }
}

export function deactivate() {
  SoundRegistry.unregister(Object.keys(BUNDLED));
}
