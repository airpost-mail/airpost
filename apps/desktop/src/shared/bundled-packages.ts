// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import fs from 'fs';
import path from 'path';

// The packages shipped with the app, under <resourcePath>: features/ always load, plugins/ can be
// turned off (`isOptional`), themes/ hold the ui-* themes.
export const BUNDLED_PACKAGE_DIRS = ['features', 'plugins', 'themes'];

export function bundledPackageRoots(resourcePath: string) {
  return BUNDLED_PACKAGE_DIRS.map((dir) => path.join(resourcePath, dir));
}

// Every bundled package directory, sorted by name across the three roots, so activation and spec
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
  return entries.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0)).map((e) => e.dir);
}
