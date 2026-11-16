// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import fs from 'fs';
import path from 'path';

// The packages shipped with the app, under <resourcePath>: plugins/ can be turned off
// (`isOptional`), themes/ hold the ui-* themes.
export const BUNDLED_PACKAGE_DIRS = ['plugins', 'themes'];

// The built-in features: app code under src/, one directory each, listed in
// src/features/index.ts. They always load, interleaved by name with the bundled packages.
export function featuresRoot(resourcePath: string) {
  return path.join(resourcePath, 'src', 'features');
}

export function bundledPackageRoots(resourcePath: string) {
  return BUNDLED_PACKAGE_DIRS.map((dir) => path.join(resourcePath, dir));
}

// Where `airpost-asset://<dir>/…` assets and the core LESS reload look: the features, then the
// bundled package roots.
export function bundledAssetRoots(resourcePath: string) {
  return [featuresRoot(resourcePath), ...bundledPackageRoots(resourcePath)];
}

export function compareNames(a: string, b: string) {
  return a < b ? -1 : a > b ? 1 : 0;
}

// Every bundled package directory, sorted by name across the roots, so activation and spec
// order don't depend on which root a package is in.
export function bundledPackagePaths(resourcePath: string) {
  const entries: { name: string; dir: string }[] = [];
  for (const root of bundledPackageRoots(resourcePath)) {
    let names: string[] = [];
    try {
      names = fs.readdirSync(root);
    } catch (err) {
      continue;
    }
    for (const name of names) {
      entries.push({ name, dir: path.join(root, name) });
    }
  }
  return entries.sort((a, b) => compareNames(a.name, b.name)).map((e) => e.dir);
}
