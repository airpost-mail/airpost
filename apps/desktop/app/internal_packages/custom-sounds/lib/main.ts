import { SoundRegistry } from 'mailspring-exports';

// The sounds are synthesized by scripts/generate-sounds.sh in apps/desktop.
const SOUNDS = {
  send: 'mailspring://custom-sounds/sounds/send.ogg',
  'hit-send': 'mailspring://custom-sounds/sounds/hit-send.ogg',
  'new-mail': 'mailspring://custom-sounds/sounds/new-mail.ogg',
};

export function activate() {
  for (const [name, url] of Object.entries(SOUNDS)) {
    SoundRegistry.register(name, url);
  }
}

export function deactivate() {
  SoundRegistry.unregister(Object.keys(SOUNDS));
}
