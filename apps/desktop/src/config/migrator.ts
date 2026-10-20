import Config from './config';

export default class ConfigMigrator {
  config: Config;

  constructor(config: Config) {
    this.config = config;
  }

  migrate() {
    this.moveEmailRenderMode();
    this.moveCalendarSettings();
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

  // The calendar's hidden calendars and its day/week/month view, kept under `mailspring.` by
  // older builds and by imported Mailspring profiles.
  moveCalendarSettings() {
    const moves = {
      'mailspring.disabledCalendars': 'core.calendar.disabledCalendars',
      'mailspring.calendarView': 'core.calendar.view',
    };
    for (const [legacyKey, key] of Object.entries(moves)) {
      const legacy = this.config.get(legacyKey);
      if (legacy == null) continue;
      this.config.set(key, legacy);
      this.config.unset(legacyKey);
    }
  }
}
