import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import NativeNotifications from '../window/platform/native-notifications';
import { systemSoundDirs } from '../platform/system-sounds';
import { aiffToWav } from '../utils/aiff';
import { isFileURLWithin } from '../utils/file-url';

const MediaTypes: { [ext: string]: string } = {
  '.aiff': 'audio/wav',
  '.aif': 'audio/wav',
  '.wav': 'audio/wav',
  '.oga': 'audio/ogg',
  '.ogg': 'audio/ogg',
};

const systemSoundBlobs = new Map<string, Promise<string | null>>();

// The OS's sounds play from blob: URLs: the app's CSP keeps file: out of media-src, and AIFF
// has to be rewritten as WAV first (utils/aiff.ts).
function systemSoundBlobURL(fileURL: string): Promise<string | null> {
  let blob = systemSoundBlobs.get(fileURL);
  if (!blob) {
    blob = (async () => {
      const file = fileURLToPath(fileURL);
      const type = MediaTypes[path.extname(file).toLowerCase()];
      if (!type) return null;
      const raw = await fs.promises.readFile(file);
      const data = type === 'audio/wav' && !file.endsWith('.wav') ? aiffToWav(raw) : raw;
      return data ? URL.createObjectURL(new Blob([data], { type })) : null;
    })().catch(() => null);
    systemSoundBlobs.set(fileURL, blob);
  }
  return blob;
}

async function playableURL(src: string): Promise<string | null> {
  // Only package-asset URLs and the OS's own sounds play; there is no bundled sounds
  // directory to resolve a bare filename against.
  if (/^(airpost-asset|mailspring):\/\//.test(src)) return src;
  if (isFileURLWithin(src, systemSoundDirs())) return systemSoundBlobURL(src);
  return null;
}

/** Plays a sound URL as SoundRegistry would, without the Do Not Disturb check (for previews). */
export async function playSoundURL(src: string, label = src) {
  const url = await playableURL(src);
  if (!url) {
    console.warn(`SoundRegistry: Could not play ${label}: unsupported source`);
    return;
  }
  const a = new Audio(url);
  // play() rejects on every call when the platform can't decode the file. A missed sound
  // effect isn't worth reporting.
  await a.play().catch((err) => {
    console.warn(`SoundRegistry: Could not play ${label}: ${err.message}`);
  });
}

class SoundRegistry {
  // A package-asset URL, a file: URL of an OS sound (platform/system-sounds.ts), or path
  // segments under the resource path.
  private _sounds: { [name: string]: string | string[] | undefined } = {};

  async playSound(name: string) {
    if (AppEnv.inSpecMode()) {
      return;
    }
    if (await NativeNotifications.doNotDisturb()) {
      return;
    }
    const src = this._sounds[name];
    if (!src) {
      return;
    }
    if (typeof src === 'string') {
      await playSoundURL(src, name);
    } else if (src instanceof Array) {
      const a = new Audio(path.join(AppEnv.getLoadSettings().resourcePath, ...src));
      a.play().catch((err) => {
        console.warn(`SoundRegistry: Could not play ${name}: ${err.message}`);
      });
    }
  }

  register(name: string | { [key: string]: string[] }, rpath?: string) {
    if (typeof name === 'object') {
      for (const [key, kpath] of Object.entries(name)) {
        this._sounds[key] = kpath;
      }
    } else if (typeof name === 'string') {
      this._sounds[name] = rpath;
    }
  }

  unregister(name: string[] | string) {
    if (name instanceof Array) {
      for (const key of name) {
        delete this._sounds[key];
      }
    } else if (typeof name === 'string') {
      delete this._sounds[name];
    }
  }
}

export default new SoundRegistry();
