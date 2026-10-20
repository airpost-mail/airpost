import path from 'path';
import fs from 'fs';
import semver from 'semver';
import { PLUGIN_API_VERSION } from './plugin-api-version';
import { applyModuleConfig, loadKeymaps, loadMenus, loadStylesheets } from './package-resources';

class NoPackageJSONError extends Error {}
class InvalidPackageNameError extends Error {}

const VALID_PACKAGE_NAME = /^[a-zA-Z0-9._-]+$/;

export function isValidPackageName(name: unknown): name is string {
  return (
    typeof name === 'string' &&
    name.length > 0 &&
    name !== '.' &&
    name !== '..' &&
    VALID_PACKAGE_NAME.test(name)
  );
}

export default class Package {
  static NoPackageJSONError = NoPackageJSONError;
  static InvalidPackageNameError = InvalidPackageNameError;

  public disposables = [];
  public directory: string;
  public name: string;
  public displayName: string;
  public syncInit: boolean;
  public windowTypes: { [windowName: string]: boolean };
  private json: any;

  // `json` stands in for the directory's package.json (built-in features have none).
  constructor(dir: string, json?: object) {
    this.directory = dir;

    if (json) {
      this.json = json;
    } else {
      let jsonString = null;
      try {
        jsonString = fs.readFileSync(path.join(dir, 'package.json')).toString();
      } catch (err) {
        // silently fail, not a file
      }
      if (!jsonString) {
        throw new NoPackageJSONError();
      }
      this.json = JSON.parse(jsonString);
    }
    if (!isValidPackageName(this.json.name)) {
      throw new InvalidPackageNameError(
        `Plugin in ${dir} has an invalid or missing "name" field. Names must match /^[a-zA-Z0-9._-]+$/.`
      );
    }
    this.name = this.json.name;
    this.displayName = this.json.displayName || this.json.name;
    this.syncInit = this.json.syncInit;
    this.windowTypes = this.json.windowTypes || { default: true };
  }

  activate() {
    const start = Date.now(); // eslint-disable-line

    this.loadKeymaps();
    this.loadMenus();
    this.loadStylesheets();

    if (this.json.main) {
      const root = path.join(this.directory, this.json.main);
      const module = require(root); // eslint-disable-line

      module.activate();
      applyModuleConfig(this.name, module);
    }

    // Uncomment to enable timing inspection
    // console.log(`Loading ${this.name} took ${Date.now() - start}`);
  }

  deactivate() {
    for (const d of this.disposables) {
      d.dispose();
    }
    this.disposables = [];

    if (this.json.main) {
      const root = path.join(this.directory, this.json.main);
      require(root).deactivate(); // eslint-disable-line
    }
  }

  isTheme() {
    return !!this.json.theme;
  }

  /** A theme's `scheme`: whether its content surfaces are light or dark. */
  scheme(): 'light' | 'dark' | null {
    const { scheme } = this.json;
    return scheme === 'light' || scheme === 'dark' ? scheme : null;
  }

  /** A theme's `sidebarScheme`, for a sidebar that differs from the rest (Darkside). */
  sidebarScheme(): 'light' | 'dark' | null {
    const { sidebarScheme } = this.json;
    return sidebarScheme === 'light' || sidebarScheme === 'dark' ? sidebarScheme : null;
  }

  isOptional() {
    return !!this.json.isOptional;
  }

  engineRange(): string | null {
    const range = this.json.engines?.mailspring;
    return typeof range === 'string' ? range : null;
  }

  /** Whether `engines.mailspring` is a range the plugin API version satisfies. A missing
   * `engines` or an invalid range is false, never a throw. */
  isEngineSet() {
    const range = this.engineRange();
    return range !== null && semver.satisfies(PLUGIN_API_VERSION, range);
  }

  isDefault() {
    return !!this.json.isDefault;
  }

  /** `tern.engineMethods`: the local-only engine methods the plugin calls (src/engine/engine-api.ts). */
  engineMethods(): string[] {
    const list = this.json.tern?.engineMethods;
    return Array.isArray(list) ? list.filter((m) => typeof m === 'string') : [];
  }

  /** `tern.service`: the background program `tern service install` runs as a user unit
   * (docs/desktop-client-plan.md T19). Only its presence is checked here; tern validates it. */
  serviceManifest(): { bin: unknown; args?: unknown } | null {
    const service = this.json.tern?.service;
    return service && typeof service === 'object' && !Array.isArray(service) ? service : null;
  }

  version(): string | null {
    return typeof this.json.version === 'string' ? this.json.version : null;
  }

  getStylesheetsPath() {
    return path.join(this.directory, 'styles');
  }

  loadKeymaps() {
    loadKeymaps(this.directory, this.disposables);
  }

  loadStylesheets() {
    loadStylesheets(this.getStylesheetsPath(), { isTheme: this.isTheme() }, this.disposables);
  }

  loadMenus() {
    loadMenus(this.directory, this.disposables);
  }
}
