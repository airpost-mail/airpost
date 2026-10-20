import Config from '../config';

export default class ConfigMigrator {
  config: Config;

  constructor(config: Config) {
    this.config = config;
  }

  migrate() {
    this.moveEmailRenderMode();
  }

  // The email colour setting moved from Reading to Appearance. 'light' was the opt-out of the
  // earlier inverted dark rendering of email bodies.
  moveEmailRenderMode() {
    const legacyKey = 'core.reading.emailRenderMode';
    const legacy = this.config.get(legacyKey);
    if (legacy == null) return;
    const mode = legacy === 'light' ? 'original' : legacy;
    if (mode === 'auto' || mode === 'dark' || mode === 'original') {
      this.config.set('core.appearance.emailRenderMode', mode);
    }
    this.config.unset(legacyKey);
  }
}
