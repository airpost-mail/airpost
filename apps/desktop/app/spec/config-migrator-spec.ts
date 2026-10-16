import ConfigMigrator from '../src/browser/config-migrator';

describe('ConfigMigrator', function () {
  const migrate = (legacy: unknown) => {
    const values: { [key: string]: unknown } = { 'core.reading.emailRenderMode': legacy };
    const config = {
      get: (key: string) => values[key],
      set: (key: string, value: unknown) => {
        values[key] = value;
      },
      unset: (key: string) => {
        delete values[key];
      },
    };
    new ConfigMigrator(config as any).migrate();
    return values;
  };

  it('moves the email colour setting to Appearance', () => {
    expect(migrate('dark')).toEqual({ 'core.appearance.emailRenderMode': 'dark' });
    expect(migrate('original')).toEqual({ 'core.appearance.emailRenderMode': 'original' });
  });

  it('turns the old light mode into original colours', () => {
    expect(migrate('light')).toEqual({ 'core.appearance.emailRenderMode': 'original' });
  });

  it('leaves the new setting alone when there is nothing to move', () => {
    expect(migrate(undefined)).toEqual({ 'core.reading.emailRenderMode': undefined });
  });
});
