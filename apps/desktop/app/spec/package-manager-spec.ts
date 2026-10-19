import fs from 'fs';
import os from 'os';
import path from 'path';
import { apiVersion } from 'mailspring-exports';
import PackageManager from '../src/package-manager';
import Package from '../src/package';
import { PLUGIN_API_VERSION } from '../src/plugin-api-version';

// Spec mode reads packages from <resourcePath>/spec/fixtures/packages, so each test gets a
// temporary resource path holding only the packages it writes.
describe('PackageManager engines check', function () {
  let resourcePath: string;

  function writePackage(name: string, json: object) {
    const dir = path.join(resourcePath, 'spec', 'fixtures', 'packages', name);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ name, ...json }));
  }

  function manager() {
    return new PackageManager({
      configDirPath: path.join(resourcePath, 'config'),
      devMode: false,
      safeMode: false,
      resourcePath,
      specMode: true,
    });
  }

  beforeEach(() => {
    resourcePath = fs.mkdtempSync(path.join(os.tmpdir(), 'tern-package-manager-'));
    spyOn(console, 'error');
  });

  afterEach(() => {
    fs.rmSync(resourcePath, { recursive: true, force: true });
  });

  it('skips a package without engines and still activates the ones after it', () => {
    writePackage('a-no-engines', { syncInit: true });
    writePackage('b-any', { syncInit: true, engines: { mailspring: '*' } });
    writePackage('c-caret', { syncInit: true, engines: { mailspring: '^1.0.0' } });

    const pm = manager();
    expect(() => pm.activatePackages('default')).not.toThrow();

    expect(pm.isPackageActive('a-no-engines')).toBe(false);
    expect(pm.isPackageActive('b-any')).toBe(true);
    expect(pm.isPackageActive('c-caret')).toBe(true);
  });

  it('skips a package whose range the plugin API version does not satisfy', () => {
    writePackage('future', { syncInit: true, engines: { mailspring: '>=2.0.0' } });
    writePackage('garbage', { syncInit: true, engines: { mailspring: 'not a range' } });
    writePackage('engines-without-mailspring', { syncInit: true, engines: { node: '>=18' } });
    writePackage('current', { syncInit: true, engines: { mailspring: `>=${PLUGIN_API_VERSION}` } });

    const pm = manager();
    pm.activatePackages('default');

    expect(pm.isPackageActive('future')).toBe(false);
    expect(pm.isPackageActive('garbage')).toBe(false);
    expect(pm.isPackageActive('engines-without-mailspring')).toBe(false);
    expect(pm.isPackageActive('current')).toBe(true);
    const messages = (console.error as jasmine.Spy).calls.map((c) => String(c.args[0]));
    expect(messages.some((m) => m.includes('future') && m.includes('>=2.0.0'))).toBe(true);
  });

  it('reads engines.mailspring without throwing when engines is missing', () => {
    writePackage('bare', {});
    const pkg = new Package(path.join(resourcePath, 'spec', 'fixtures', 'packages', 'bare'));
    expect(pkg.engineRange()).toBe(null);
    expect(pkg.isEngineSet()).toBe(false);
  });

  it('exports the plugin API version', () => {
    expect(apiVersion).toBe(PLUGIN_API_VERSION);
    expect(apiVersion).toBe('1.30.0');
  });

  it('reads tern.engineMethods, keeping only strings', () => {
    writePackage('declares', { tern: { engineMethods: ['account.test', 7, 'secrets.set'] } });
    const pkg = new Package(path.join(resourcePath, 'spec', 'fixtures', 'packages', 'declares'));
    expect(pkg.engineMethods()).toEqual(['account.test', 'secrets.set']);
    writePackage('declares-nothing', { tern: { engineMethods: 'account.test' } });
    const none = new Package(
      path.join(resourcePath, 'spec', 'fixtures', 'packages', 'declares-nothing')
    );
    expect(none.engineMethods()).toEqual([]);
  });
});
