import fs from 'fs';
import os from 'os';
import path from 'path';
import PackageManager from '../../src/packages/manager';
import BuiltinFeature from '../../src/packages/builtin-feature';
import { FEATURES } from '../../src/features';
import { thirdPartyPlugins } from '../../src/features/preferences/tabs/plugins/plugins';

// Features and bundled plugins activate as one list sorted by directory name, and the
// registries render in registration order, so this order decides where toolbar buttons,
// message:BodyHeader banners and composer extensions appear. These are the lists from when the
// features were packages in features/.
const ORDER = {
  default: {
    sync: [
      'account-sidebar',
      'attachments',
      'composer',
      'custom-fonts',
      'main-calendar',
      'message-autoload-images',
      'message-list',
      'notifications',
      'preferences',
      'thread-list',
      'thread-search',
    ],
    deferred: [
      'activity',
      'ai',
      'category-mapper',
      'category-picker',
      'composer-grammar-check',
      'composer-signature',
      'composer-templates',
      'custom-sounds',
      'draft-list',
      'events',
      'link-tracking',
      'unsubscribe',
      'mode-switch',
      'open-tracking',
      'participant-profile',
      'phishing-detection',
      'print',
      'remove-tracking-pixels',
      'send-and-archive',
      'send-later',
      'send-reminders',
      'system-tray',
      'theme-picker',
      'thread-sharing',
      'thread-snooze',
      'translation',
      'undo-redo',
      'unread-notifications',
    ],
  },
  composer: {
    sync: ['attachments', 'composer', 'custom-fonts'],
    deferred: [
      'ai',
      'composer-grammar-check',
      'composer-signature',
      'composer-templates',
      'custom-sounds',
      'link-tracking',
      'open-tracking',
      'phishing-detection',
      'send-and-archive',
      'send-later',
      'send-reminders',
      'translation',
    ],
  },
  'thread-popout': {
    sync: ['attachments', 'composer', 'custom-fonts', 'message-autoload-images', 'message-list'],
    deferred: [
      'ai',
      'composer-grammar-check',
      'composer-signature',
      'composer-templates',
      'custom-sounds',
      'events',
      'link-tracking',
      'unsubscribe',
      'open-tracking',
      'phishing-detection',
      'print',
      'remove-tracking-pixels',
      'send-and-archive',
      'send-later',
      'translation',
      'undo-redo',
    ],
  },
  contacts: { sync: ['contacts', 'custom-fonts'], deferred: ['custom-sounds'] },
  onboarding: { sync: ['custom-fonts'], deferred: ['custom-sounds', 'onboarding'] },
  emptyWindow: { sync: ['custom-fonts'], deferred: ['custom-sounds'] },
};

describe('Built-in features', function () {
  const resourcePath = AppEnv.getLoadSettings().resourcePath;
  let configDirPath: string;

  function manager({ safeMode = true } = {}) {
    return new PackageManager({
      configDirPath,
      devMode: false,
      safeMode,
      resourcePath,
      specMode: false,
    });
  }

  beforeEach(() => {
    configDirPath = fs.mkdtempSync(path.join(os.tmpdir(), 'tern-builtin-features-'));
  });

  afterEach(() => {
    fs.rmSync(configDirPath, { recursive: true, force: true });
  });

  it('lists every directory in src/features, in name order', () => {
    const root = path.join(resourcePath, 'src', 'features');
    const dirs = fs
      .readdirSync(root)
      .filter((name) => fs.statSync(path.join(root, name)).isDirectory())
      .sort();
    expect(FEATURES.map((f) => f.dir)).toEqual(dirs);
    // Each has an entry module; `load` requires it lazily, so check it resolves without running it.
    for (const dir of dirs) {
      expect(() => require.resolve(path.join(root, dir))).not.toThrow();
    }
  });

  Object.keys(ORDER).forEach((windowType) => {
    it(`activates features and plugins in the same order as before in ${windowType}`, () => {
      const pm = manager();
      const activated: string[] = [];
      spyOn(pm, 'activatePackage').andCallFake((pkg) => {
        if (!pkg.isTheme()) activated.push(pkg.name);
      });
      pm.activatePackages(windowType);
      expect(activated).toEqual(ORDER[windowType].sync);
      activated.length = 0;
      advanceClock(2500);
      expect(activated).toEqual(ORDER[windowType].deferred);
    });
  });

  it('keeps the package names, and never offers a feature as optional', () => {
    const pm = manager();
    const unsubscribe = pm.getPackageNamed('unsubscribe');
    expect(unsubscribe instanceof BuiltinFeature).toBe(true);
    expect(unsubscribe.directory).toBe(
      path.join(resourcePath, 'src', 'features', 'list-unsubscribe')
    );
    for (const feature of FEATURES) {
      const pkg = pm.getPackageNamed(feature.name || feature.dir);
      expect(pkg instanceof BuiltinFeature).toBe(true);
      expect(pkg.isOptional()).toBe(false);
      expect(pkg.isTheme()).toBe(false);
      expect(pkg.isEngineSet()).toBe(true);
      expect(pkg.directory.startsWith(resourcePath)).toBe(true);
    }
  });

  it('is not discovered in spec mode', () => {
    const pm = new PackageManager({
      configDirPath,
      devMode: false,
      safeMode: false,
      resourcePath,
      specMode: true,
    });
    expect(pm.getPackageNamed('composer')).toBeUndefined();
  });

  it('lets a user package with the same name replace a feature in place', () => {
    const dir = path.join(configDirPath, 'packages', 'composer');
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(
      path.join(dir, 'package.json'),
      JSON.stringify({ name: 'composer', engines: { mailspring: '*' } })
    );
    const before = Object.keys(manager().available);
    const pm = manager({ safeMode: false });
    expect(Object.keys(pm.available)).toEqual(before);
    expect(pm.getPackageNamed('composer') instanceof BuiltinFeature).toBe(false);
    expect(pm.getPackageNamed('composer').directory).toBe(dir);
  });

  it('lists user plugins in Preferences > Plugins, and no feature or bundled plugin', () => {
    const dir = path.join(configDirPath, 'packages', 'third-party');
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(
      path.join(dir, 'package.json'),
      JSON.stringify({ name: 'third-party', engines: { mailspring: '*' } })
    );
    const pm = manager({ safeMode: false });
    spyOn(AppEnv.packages, 'getAvailablePackages').andReturn(pm.getAvailablePackages());
    expect(thirdPartyPlugins().map((row) => row.name)).toEqual(['third-party']);
  });

  describe('activate', () => {
    let root: string;
    let mod: any;

    beforeEach(() => {
      root = fs.mkdtempSync(path.join(os.tmpdir(), 'tern-builtin-feature-'));
      const dir = path.join(root, 'src', 'features', 'fake');
      fs.mkdirSync(path.join(dir, 'keymaps'), { recursive: true });
      fs.mkdirSync(path.join(dir, 'styles'), { recursive: true });
      fs.writeFileSync(path.join(dir, 'keymaps', 'fake.json'), '{}');
      fs.writeFileSync(path.join(dir, 'styles', 'fake.css'), '.fake {}');
      mod = {
        activate: jasmine.createSpy('activate').andCallFake(function () {
          this.state = 'kept';
        }),
        deactivate: jasmine.createSpy('deactivate'),
        config: { enabled: { type: 'boolean', default: true } },
      };
    });

    afterEach(() => {
      fs.rmSync(root, { recursive: true, force: true });
    });

    it('loads keymaps and styles, calls activate on the module and sets its config schema', () => {
      const keymap = { dispose: jasmine.createSpy('keymap') };
      const style = { dispose: jasmine.createSpy('style') };
      spyOn(AppEnv.keymaps, 'loadKeymap').andReturn(keymap);
      spyOn(AppEnv.styles, 'addStyleSheet').andReturn(style);
      spyOn(AppEnv.config, 'setSchema');

      const feature = new BuiltinFeature(root, {
        dir: 'fake',
        name: 'fake-name',
        windowTypes: { default: true },
        load: () => mod,
      });
      feature.activate();

      expect(AppEnv.keymaps.loadKeymap).toHaveBeenCalledWith(
        path.join(root, 'src', 'features', 'fake', 'keymaps', 'fake.json')
      );
      const [content, options] = (AppEnv.styles.addStyleSheet as jasmine.Spy).calls[0].args;
      expect(content).toBe('.fake {}');
      expect(options).toEqual({
        sourcePath: path.join(root, 'src', 'features', 'fake', 'styles', 'fake.css'),
        priority: 0,
      });
      expect(mod.activate).toHaveBeenCalled();
      expect(mod.state).toBe('kept');
      expect(AppEnv.config.setSchema).toHaveBeenCalledWith('fake-name', {
        type: 'object',
        properties: mod.config,
      });

      feature.deactivate();
      expect(keymap.dispose).toHaveBeenCalled();
      expect(style.dispose).toHaveBeenCalled();
      expect(mod.deactivate).toHaveBeenCalled();
    });
  });
});
