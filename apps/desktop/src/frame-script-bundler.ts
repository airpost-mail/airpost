import fs from 'fs';
import path from 'path';
import { builtinModules } from 'module';

// Packs a CommonJS module graph into one script for a document without Node (the message
// frame). The app has no bundler: in development the sources are .ts, transpiled here the
// way compile-cache-ts does; packaged builds ship them as .js already. Node built-ins are
// refused, so a module that needs Node can't end up in the frame by accident.

const RequirePattern = /\brequire\((["'])([^"']+)\1\)/g;

const CompilerOptions = {
  target: 99, // ESNext
  module: 1, // CommonJS
  jsx: 2, // React
  esModuleInterop: true,
  experimentalDecorators: true,
  useDefineForClassFields: false,
  removeComments: true,
};

function transpile(filePath: string) {
  const source = fs.readFileSync(filePath, 'utf8');
  if (!/\.tsx?$/.test(filePath)) return source;
  // Dev only: packaged builds ship .js, so the pruned devDependency is never needed there.
  // eslint-disable-next-line import/no-extraneous-dependencies
  const TypeScript = require('typescript');
  return TypeScript.transpileModule(source, {
    compilerOptions: CompilerOptions,
    fileName: filePath,
  }).outputText;
}

function resolveFrom(
  fromFile: string,
  specifier: string,
  aliases: { [spec: string]: string | null }
) {
  if (aliases[specifier]) return aliases[specifier];
  const bare = specifier.replace(/^node:/, '');
  if (specifier.startsWith('node:') || builtinModules.includes(bare.split('/')[0])) {
    throw new Error(`${fromFile} requires the Node module ${specifier}`);
  }
  return require.resolve(specifier, { paths: [path.dirname(fromFile)] });
}

/**
 * `aliases` maps a specifier to the file to bundle in its place, or to null for a module the
 * bundled code only requires lazily on paths the frame never takes (requiring it throws).
 */
export function bundleScript(entryPath: string, aliases: { [spec: string]: string | null } = {}) {
  const ids = new Map<string, number>();
  const modules: string[] = [];

  const stub = (specifier: string) => {
    const key = `stub:${specifier}`;
    if (!ids.has(key)) {
      ids.set(key, modules.length);
      modules.push(
        `[function () { throw new Error(${JSON.stringify(`${specifier} is not bundled`)}); }, {}]`
      );
    }
    return ids.get(key);
  };

  const visit = (filePath: string): number => {
    if (ids.has(filePath)) return ids.get(filePath);
    const id = modules.length;
    ids.set(filePath, id);
    modules.push('');
    const code = transpile(filePath);
    const deps: { [spec: string]: number } = {};
    for (const match of code.matchAll(RequirePattern)) {
      const specifier = match[2];
      if (specifier in deps) continue;
      deps[specifier] =
        aliases[specifier] === null
          ? stub(specifier)
          : visit(resolveFrom(filePath, specifier, aliases));
    }
    modules[id] = `[function (require, module, exports) {\n${code}\n}, ${JSON.stringify(deps)}]`;
    return id;
  };
  visit(path.resolve(entryPath));

  return `(function () {
var defs = [${modules.join(',\n')}];
var cache = {};
function load(id) {
  if (cache[id]) return cache[id].exports;
  var module = (cache[id] = { exports: {} });
  var deps = defs[id][1];
  defs[id][0].call(module.exports, function (spec) {
    if (!(spec in deps)) throw new Error('module not bundled: ' + spec);
    return load(deps[spec]);
  }, module, module.exports);
  return module.exports;
}
load(0);
})();
`;
}
