// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

// Types for the untyped modules LessCompileCache uses, as far as it uses them.

declare module 'less-cache' {
  interface LessCacheOptions {
    cacheDir: string;
    fallbackDir?: string;
    importPaths?: string[];
    resourcePath?: string;
    syncCaches?: boolean;
  }

  class LessCache {
    constructor(options: LessCacheOptions);
    importPaths: string[];
    setImportPaths(importPaths?: string[]): void;
    parseLess(filePath: string, contents: string): { imports: unknown; css: string };
    readFileSync(absoluteFilePath: string): string;
    cssForFile(filePath: string, lessContent: string, digest?: string): string;
  }

  export = LessCache;
}

declare module 'rtlcss' {
  const rtlcss: { process(css: string): string };
  export = rtlcss;
}
