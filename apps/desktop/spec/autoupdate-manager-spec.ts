import AutoUpdateManager from '../src/browser/autoupdate-manager';

describe('AutoUpdateManager', function () {
  beforeEach(function () {
    this.specMode = true;
    this.config = {
      set: jasmine.createSpy('config.set'),
      get: () => undefined,
    };
  });

  describe('without a feed', () =>
    it('makes no requests and reports updates as unsupported', function () {
      spyOn(AutoUpdateManager.prototype, 'setupAutoUpdater');
      const m = new AutoUpdateManager('3.222.1', this.config, this.specMode, '');
      advanceClock(10);
      expect(AutoUpdateManager.prototype.setupAutoUpdater).not.toHaveBeenCalled();
      expect(m.feedURL).toBe(null);
      expect(m.getState()).toEqual('unsupported');
      expect(() => {
        m.check();
        m.install();
      }).not.toThrow();
    }));

  describe('with attached commit version', () =>
    it('correctly sets the feedURL', function () {
      const m = new AutoUpdateManager(
        '3.222.1-abc',
        this.config,
        this.specMode,
        'https://updates.example.com/'
      );
      spyOn(m, 'setupAutoUpdater');
      expect(m.feedURL).toEqual(
        'https://updates.example.com/check/' +
          process.platform +
          '/' +
          process.arch +
          '/3.222.1-abc/anonymous/stable'
      );
    }));

  describe('with no attached commit', () =>
    it('correctly sets the feedURL', function () {
      const m = new AutoUpdateManager(
        '3.222.1',
        this.config,
        this.specMode,
        'https://updates.example.com'
      );
      spyOn(m, 'setupAutoUpdater');
      expect(m.feedURL).toEqual(
        'https://updates.example.com/check/' +
          process.platform +
          '/' +
          process.arch +
          '/3.222.1/anonymous/stable'
      );
    }));
});
