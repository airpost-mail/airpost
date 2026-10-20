import fs from 'fs';
import os from 'os';
import path from 'path';
import {
  PLUGIN_INSTALLS_KEY,
  hashPackageDir,
  pluginInstallRecord,
  recordPluginInstall,
} from '../../src/packages/install-record';

describe('plugin install records', () => {
  let dir: string;
  let stored: any;

  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tern-plugin-hash-'));
    fs.mkdirSync(path.join(dir, 'lib'));
    fs.writeFileSync(path.join(dir, 'package.json'), '{"name":"p"}');
    fs.writeFileSync(path.join(dir, 'lib', 'main.js'), 'module.exports = {};');
    stored = undefined;
    const realGet = AppEnv.config.get.bind(AppEnv.config);
    spyOn(AppEnv.config, 'get').andCallFake((key: string) =>
      key === PLUGIN_INSTALLS_KEY ? stored : realGet(key)
    );
    spyOn(AppEnv.config, 'set').andCallFake((key: string, value: any) => {
      if (key === PLUGIN_INSTALLS_KEY) stored = value;
    });
  });

  afterEach(() => {
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('hashes paths and contents, the same tree always the same', () => {
    const first = hashPackageDir(dir);
    expect(first).toMatch(/^[0-9a-f]{64}$/);
    expect(hashPackageDir(dir)).toBe(first);

    fs.writeFileSync(path.join(dir, 'lib', 'main.js'), 'module.exports = {x: 1};');
    const edited = hashPackageDir(dir);
    expect(edited).not.toBe(first);

    fs.renameSync(path.join(dir, 'lib', 'main.js'), path.join(dir, 'lib', 'other.js'));
    expect(hashPackageDir(dir)).not.toBe(edited);
  });

  it('hashes a symlink by its target without following it', () => {
    fs.symlinkSync('/etc/hosts', path.join(dir, 'link'));
    const linked = hashPackageDir(dir);
    fs.unlinkSync(path.join(dir, 'link'));
    fs.symlinkSync('/etc/passwd', path.join(dir, 'link'));
    expect(hashPackageDir(dir)).not.toBe(linked);
  });

  it('records the source and hash per plugin', () => {
    stored = { other: { source: '/x', sha256: 'abc', installedAt: 1 } };
    const record = recordPluginInstall('p', '/Users/me/Downloads/p', dir);
    expect(record.source).toBe('/Users/me/Downloads/p');
    expect(record.sha256).toBe(hashPackageDir(dir));
    expect(pluginInstallRecord('p')).toEqual(record);
    expect(pluginInstallRecord('other').sha256).toBe('abc');
    expect(pluginInstallRecord('missing')).toBe(null);
  });
});
