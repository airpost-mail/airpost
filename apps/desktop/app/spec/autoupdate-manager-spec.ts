import AutoUpdateManager from '../src/browser/autoupdate-manager';

describe('AutoUpdateManager', function () {
  beforeEach(function () {
    this.specMode = true;
    this.config = {
      set: jasmine.createSpy('config.set'),
      get: (key) => {
        if (key === 'env') {
          return 'production';
        }
      },
    };
  });

  describe('with attached commit version', () =>
    it('correctly sets the feedURL', function () {
      const m = new AutoUpdateManager('3.222.1-abc', this.config, this.specMode);
      spyOn(m, 'setupAutoUpdater');
      expect(m.feedURL).toEqual(
        'https://updates.getmailspring.com/check/' +
          process.platform +
          '/' +
          process.arch +
          '/3.222.1-abc/anonymous/stable'
      );
    }));

  describe('with no attached commit', () =>
    it('correctly sets the feedURL', function () {
      const m = new AutoUpdateManager('3.222.1', this.config, this.specMode);
      spyOn(m, 'setupAutoUpdater');
      expect(m.feedURL).toEqual(
        'https://updates.getmailspring.com/check/' +
          process.platform +
          '/' +
          process.arch +
          '/3.222.1/anonymous/stable'
      );
    }));
});
