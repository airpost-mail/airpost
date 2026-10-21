import SoundRegistry from '../../packages/sound-registry';

// The sounds are synthesized by scripts/generate-sounds.sh in apps/desktop.
const SOUNDS = {
  send: 'tern-asset://custom-sounds/sounds/send.ogg',
  'hit-send': 'tern-asset://custom-sounds/sounds/hit-send.ogg',
  'new-mail': 'tern-asset://custom-sounds/sounds/new-mail.ogg',
};

export function activate() {
  for (const [name, url] of Object.entries(SOUNDS)) {
    SoundRegistry.register(name, url);
  }
}

export function deactivate() {
  SoundRegistry.unregister(Object.keys(SOUNDS));
}
