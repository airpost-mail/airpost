import Config from '../config';

export default class ConfigMigrator {
  config: Config;

  constructor(config: Config) {
    this.config = config;
  }

  migrate() {
    // 'light' was the opt-out of the earlier inverted dark rendering of email bodies.
    if (this.config.get('core.reading.emailRenderMode') === 'light') {
      this.config.set('core.reading.emailRenderMode', 'original');
    }
  }
}
