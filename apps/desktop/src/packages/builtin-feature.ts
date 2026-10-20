// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import path from 'path';
import Package from './package';
import { applyModuleConfig, loadKeymaps, loadMenus, loadStylesheets } from './resources';
import { featuresRoot } from '../shared/bundled-packages';
import type { FeatureManifest } from '../features';

// A built-in feature from src/features/index.ts, held in PackageManager like a package so it
// keeps its place in the activation order, its name in AppEnv.packages and its error
// attribution. It has no package.json: what one would say comes from the manifest entry.
export default class BuiltinFeature extends Package {
  readonly builtin = true;
  private manifest: FeatureManifest;

  constructor(resourcePath: string, manifest: FeatureManifest) {
    const name = manifest.name || manifest.dir;
    super(path.join(featuresRoot(resourcePath), manifest.dir), {
      name,
      syncInit: !!manifest.syncInit,
      windowTypes: manifest.windowTypes,
      engines: { mailspring: '*' },
    });
    this.manifest = manifest;
  }

  activate() {
    loadKeymaps(this.directory, this.disposables);
    loadMenus(this.directory, this.disposables);
    loadStylesheets(this.getStylesheetsPath(), { isTheme: false }, this.disposables);

    const module = this.manifest.load();
    // Called on the module: some features keep their state on `this`.
    module.activate();
    applyModuleConfig(this.name, module);
  }

  deactivate() {
    for (const d of this.disposables) {
      d.dispose();
    }
    this.disposables = [];
    this.manifest.load().deactivate();
  }
}
