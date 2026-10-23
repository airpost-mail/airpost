// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

// The keymaps, menus and stylesheets a package or a built-in feature brings, loaded from its
// directory. Each pushes the disposables that unload what it added onto `disposables` as it
// goes, so a failure halfway still leaves the earlier ones to dispose.

import path from 'path';
import fs from 'fs';

export interface Disposable {
  dispose(): void;
}

// Every keymaps/*.json, in the `package` keymap layer.
export function loadKeymaps(directory: string, disposables: Disposable[]) {
  let keymapPaths: string[] = [];
  const keymapsRoot = path.join(directory, 'keymaps');
  try {
    keymapPaths = fs
      .readdirSync(keymapsRoot)
      .filter((fn) => fn.endsWith('.json'))
      .map((fn) => path.join(keymapsRoot, fn));
  } catch (err) {
    // no keymaps
  }

  for (const keymapPath of keymapPaths) {
    disposables.push(AppEnv.keymaps.loadKeymap(keymapPath));
  }
}

// The stylesheets in a package's styles/ folder, in load order: styles/index.* alone when there
// is one, else every *ss file, by name. Names compare with the extension as .css, so a file
// keeps its place whether it is foo.less or foo.css.
export function stylesheetsIn(stylesRoot: string, { isTheme }: { isTheme: boolean }) {
  let filenames: string[] = [];
  try {
    filenames = fs.readdirSync(stylesRoot);
  } catch (err) {
    return []; // styles directory not found
  }
  const index = filenames.find((fn) => fn.startsWith('index.'));
  if (index) return [path.join(stylesRoot, index)];

  const key = (fn: string) => fn.replace(/\.less$/, '.css');
  return (
    filenames
      // ThemeManager loads a theme's tokens itself, ahead of its other stylesheets.
      .filter((fn) => fn.endsWith('ss') && !(isTheme && fn === 'tokens.css'))
      .sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : 0))
      .map((fn) => path.join(stylesRoot, fn))
  );
}

// Loads stylesheetsIn(stylesRoot). Themes load at priority 1, after everything else.
export function loadStylesheets(
  stylesRoot: string,
  { isTheme }: { isTheme: boolean },
  disposables: Disposable[]
) {
  for (const sourcePath of stylesheetsIn(stylesRoot, { isTheme })) {
    const content = AppEnv.themes.cssContentsOfStylesheet(sourcePath);
    disposables.push(
      AppEnv.styles.addStyleSheet(content, {
        sourcePath,
        priority: isTheme ? 1 : 0,
      })
    );
  }
}

// Every menus/*.json that has a `menu` key.
export function loadMenus(directory: string, disposables: Disposable[]) {
  const menusRoot = path.join(directory, 'menus');
  let menuPaths: string[] = [];

  try {
    menuPaths = fs
      .readdirSync(menusRoot)
      .filter((fn) => fn.endsWith('.json'))
      .map((fn) => path.join(menusRoot, fn));
  } catch (err) {
    // no menus
  }

  for (const menuPath of menuPaths) {
    const content = JSON.parse(fs.readFileSync(menuPath).toString());
    if (content.menu) {
      disposables.push(AppEnv.menu.add(content.menu));
    }
  }
}

// What a package's `main` module may export besides activate/deactivate: a config schema or
// defaults under the package's name, and a hook run after them.
export function applyModuleConfig(name: string, module: any) {
  if (module.config && typeof module.config === 'object') {
    AppEnv.config.setSchema(name, { type: 'object', properties: module.config });
  } else if (module.configDefaults && typeof module.configDefaults === 'object') {
    AppEnv.config.setDefaults(name, module.configDefaults);
  }
  if (module.activateConfig) {
    module.activateConfig();
  }
}
