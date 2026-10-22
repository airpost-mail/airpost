import { pathToFileURL } from 'url';
import SoundRegistry from '../../packages/sound-registry';
import { SoundEvent, systemSoundPath } from '../../platform/system-sounds';
import * as Actions from '../../data/actions';

// Played where the OS has no fitting sound of its own (see platform/system-sounds.ts). They are
// synthesized by scripts/generate-sounds.sh in apps/desktop.
const BUNDLED: Record<SoundEvent, string> = {
  send: 'tern-asset://custom-sounds/sounds/send.ogg',
  'hit-send': 'tern-asset://custom-sounds/sounds/hit-send.ogg',
  'new-mail': 'tern-asset://custom-sounds/sounds/new-mail.ogg',
};

let teardown: Array<() => void> = [];

function register(event: SoundEvent) {
  const macName = event === 'new-mail' ? AppEnv.config.get('core.notifications.sound') : undefined;
  const file = systemSoundPath(event, macName);
  if (file === false) {
    SoundRegistry.unregister(event);
  } else {
    SoundRegistry.register(event, file ? pathToFileURL(file).href : BUNDLED[event]);
  }
}

export function activate() {
  for (const event of Object.keys(BUNDLED) as SoundEvent[]) {
    register(event);
  }
  const picked = AppEnv.config.onDidChange('core.notifications.sound', () => register('new-mail'));
  teardown.push(() => picked.dispose());
  // A composer window closes right after Send and would cut its own sound off.
  if (AppEnv.isMainWindow()) {
    teardown.push(Actions.playSound.listen((name: string) => SoundRegistry.playSound(name)));
  }
}

export function deactivate() {
  SoundRegistry.unregister(Object.keys(BUNDLED));
  teardown.forEach((fn) => fn());
  teardown = [];
}
