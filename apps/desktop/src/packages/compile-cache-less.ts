import _ from 'underscore';
import path from 'path';
import LessCache from 'less-cache';
import rtlcss from 'rtlcss';

import { isRTL } from '../intl/localize';

// The LESS import surface: `@import 'ui-variables'`, `'ui-mixins'`, `'base/ui-variables'` and
// `'mixins/…'` resolve here. The app's own stylesheets are plain CSS; this is for third-party
// packages (LESS-variables themes and v1 plugins that ship .less) and goes with v1 compatibility
// at 2.0.
export function lessSearchPaths(resourcePath: string) {
  const compat = path.join(resourcePath, 'static', 'styles', 'compat', 'less');
  return [compat, path.join(compat, 'base')];
}

// {LessCache} wrapper used by {ThemeManager} to compile third-party LESS stylesheets.
export default class LessCompileCache {
  lessSearchPaths: string[];
  cache: LessCache;

  constructor({
    configDirPath,
    resourcePath,
    importPaths = [],
  }: {
    configDirPath: string;
    resourcePath: string;
    importPaths?: string[];
  }) {
    this.lessSearchPaths = lessSearchPaths(resourcePath);

    this.cache = new LessCache({
      cacheDir: path.join(configDirPath, 'compile-cache', isRTL ? 'less-rtl' : 'less'),
      fallbackDir: path.join(resourcePath, 'less-compile-cache'),
      importPaths: importPaths.concat(this.lessSearchPaths),
      resourcePath: resourcePath,
    });

    // swizzle the LessCache parseLess method to perform rtl conversion
    // on the final CSS, and THEN save the cached result / return the CSS
    if (isRTL) {
      const original = this.cache.parseLess;
      this.cache.parseLess = (filePath: string, contents: string) => {
        const { imports, css } = original.call(this.cache, filePath, contents);
        return { imports, css: rtlcss.process(css) };
      };
    }
  }

  // Setting the import paths is a VERY expensive operation (200ms +)
  // because it walks the entire file tree and does a file state for each
  // and every importPath. If we already have the imports, then load it
  // from our backend FileListCache.
  setImportPaths(importPaths: string[] = []) {
    const fullImportPaths = importPaths.concat(this.lessSearchPaths);
    if (!_.isEqual(fullImportPaths, this.cache.importPaths)) {
      this.cache.setImportPaths(fullImportPaths);
    }
  }

  read(stylesheetPath: string) {
    return this.cache.readFileSync(stylesheetPath);
  }

  cssForFile(stylesheetPath: string, lessContent: string) {
    return this.cache.cssForFile(stylesheetPath, lessContent);
  }
}
