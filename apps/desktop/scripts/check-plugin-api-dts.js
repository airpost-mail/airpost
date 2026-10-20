#!/usr/bin/env node
// Type-checks the plugin API barrels' .d.ts files. tsconfig.json sets skipLibCheck, which also
// skips our own .d.ts files, so a path in them that no longer resolves would silently type that
// export as `any`. skipLibCheck stays off here, and only the barrels' diagnostics are reported:
// node_modules typings have errors of their own that the app's typecheck rightly skips.
const path = require('path');
const ts = require('typescript');

const appDir = path.resolve(__dirname, '..');
const barrels = process.argv.slice(2).length
  ? process.argv.slice(2).map((file) => path.resolve(file))
  : ['mailspring-exports.d.ts', 'mailspring-component-kit.d.ts'].map((file) =>
      path.join(appDir, 'src', 'plugin-api', file)
    );

const configPath = path.join(appDir, 'tsconfig.json');
const { config, error } = ts.readConfigFile(configPath, ts.sys.readFile);
if (error) {
  console.error(ts.formatDiagnostic(error, ts.createCompilerHost({})));
  process.exit(1);
}
const parsed = ts.parseJsonConfigFileContent(config, ts.sys, appDir);
const options = { ...parsed.options, skipLibCheck: false, noEmit: true };

// The ambient declarations (AppEnv, the jasmine and window globals) are roots too, so the
// barrels see the same globals as the app.
const ambient = parsed.fileNames.filter((file) =>
  file.startsWith(path.join(appDir, 'src', 'types'))
);
const program = ts.createProgram([...barrels, ...ambient], options);

const diagnostics = [];
for (const file of barrels) {
  const sourceFile = program.getSourceFile(file);
  if (!sourceFile) {
    console.error(`${file}: not found`);
    process.exit(1);
  }
  diagnostics.push(
    ...program.getSyntacticDiagnostics(sourceFile),
    ...program.getSemanticDiagnostics(sourceFile)
  );
}

if (diagnostics.length) {
  const host = {
    getCanonicalFileName: (file) => file,
    getCurrentDirectory: () => process.cwd(),
    getNewLine: () => '\n',
  };
  console.error(ts.formatDiagnostics(diagnostics, host));
  process.exit(1);
}
