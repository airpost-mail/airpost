/* eslint global-require: 0 */
/* eslint import/no-dynamic-require: 0 */
import _ from 'underscore';
import fs from 'fs';
import path from 'path';
import { bundledPackagePaths } from '../../../src/shared/bundled-packages';

// Core and bundled package tests are `*.test.*` files under a `tests/` dir. Third-party
// packages may also keep `*-spec.*` files under `specs/`.
const TEST_FILE = /\.test\.(js|jsx|ts|tsx)$/;
const SPEC_FILE = /-spec\.(js|jsx|es6|es|ts|tsx)$/;

class TestLoader {
  jasmineEnv: any;
  loadSettings: any;

  loadSpecs(loadSettings, jasmineEnv) {
    this.jasmineEnv = jasmineEnv;
    this.loadSettings = loadSettings;
    if (this.loadSettings.specDirectory) {
      const dir = this.loadSettings.specDirectory;
      this._loadSpecsInDir(dir, dir, [TEST_FILE, SPEC_FILE]);
      this._setSpecType('user');
    } else {
      this._loadAllSpecs();
    }
  }

  _loadAllSpecs() {
    const { resourcePath } = this.loadSettings;

    this._loadSpecsInDir(path.join(resourcePath, 'tests'), resourcePath, [TEST_FILE], ['support']);

    this._setSpecType('core');

    let packagePathsList: string[] = [];
    const iterable = bundledPackagePaths(resourcePath);
    for (let i = 0; i < iterable.length; i++) {
      const packagePath = iterable[i];
      if (fs.statSync(packagePath).isDirectory()) {
        packagePathsList.push(packagePath);
      }
    }

    packagePathsList = _.uniq(packagePathsList);

    const packagePaths: { [key: string]: string[] } = _.groupBy(packagePathsList, (packagePath) => {
      if (packagePath.indexOf(`${resourcePath}${path.sep}`) === 0) {
        return 'bundled';
      }
      return 'user';
    });

    // Run bundled package tests
    const iterable1 = packagePaths.bundled != null ? packagePaths.bundled : [];

    for (let j = 0; j < iterable1.length; j++) {
      const packagePath = iterable1[j];
      this._loadSpecsInDir(path.join(packagePath, 'tests'), resourcePath, [TEST_FILE]);
    }
    this._setSpecType('bundled');

    // Run user package tests
    const iterable2 = packagePaths.user != null ? packagePaths.user : [];
    for (let k = 0; k < iterable2.length; k++) {
      const packagePath = iterable2[k];
      this._loadSpecsInDir(path.join(packagePath, 'tests'), packagePath, [TEST_FILE]);
      this._loadSpecsInDir(path.join(packagePath, 'specs'), packagePath, [SPEC_FILE]);
    }
    return this._setSpecType('user');
  }

  // Requires every file under `specDirectory` whose name matches one of `suffixes` and, when
  // --spec-file-pattern (or --test-file-pattern) is given, whose path relative to `base`
  // matches it too. Files under the `skip` subdirs are never required.
  _loadSpecsInDir(specDirectory, base: string, suffixes: RegExp[], skip: string[] = []) {
    const { specFilePattern } = this.loadSettings;
    const pattern =
      typeof specFilePattern === 'string' && specFilePattern.length > 0
        ? new RegExp(specFilePattern)
        : null;

    if (!fs.existsSync(specDirectory)) return;
    for (const file of fs.readdirSync(specDirectory, { recursive: true }) as string[]) {
      if (skip.some((dir) => file === dir || file.startsWith(`${dir}${path.sep}`))) continue;
      if (!suffixes.some((suffix) => suffix.test(file))) continue;
      const specFilePath = path.join(specDirectory, file);
      if (pattern && !pattern.test(path.relative(base, specFilePath))) continue;
      try {
        require(specFilePath);
      } catch (err) {
        throw new Error(`Error requiring spec file: ${specFilePath}: ${err.toString()}`);
      }
    }

    this._setSpecDirectory(specDirectory);
  }

  _setSpecDirectory(specDirectory) {
    this._setSpecField('specDirectory', specDirectory);
  }

  _setSpecField(name, value) {
    const specs = this.jasmineEnv.currentRunner().specs();
    if (specs.length === 0) {
      return;
    }

    for (let i = 0; i < specs.length; i++) {
      if (specs[i][name]) break;
      specs[i][name] = value;
    }
  }

  _setSpecType(specType) {
    this._setSpecField('specType', specType);
  }
}
export default new TestLoader();
