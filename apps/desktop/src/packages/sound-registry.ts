import path from 'path';
import NativeNotifications from '../window/platform/native-notifications';

class SoundRegistry {
  // A package-asset URL, or path segments under the resource path.
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

    const a = new Audio();
    const { resourcePath } = AppEnv.getLoadSettings();

    if (typeof src === 'string') {
      // Only package-asset URLs play; there is no bundled sounds directory to resolve a
      // bare filename against.
      if (!/^(tern-asset|mailspring):\/\//.test(src)) return;
      a.src = src;
    } else if (src instanceof Array) {
      a.src = path.join(resourcePath, ...src);
    }
    a.autoplay = true;
    // play() rejects on every call when the platform can't decode the file. A missed sound
    // effect isn't worth reporting.
    a.play().catch((err) => {
      console.warn(`SoundRegistry: Could not play ${name}: ${err.message}`);
    });
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
