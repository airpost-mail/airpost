// SPDX-License-Identifier: GPL-3.0-only
//
// Transpiles src/ and test/ to CommonJS in a temp directory and runs the tests with node:test.
// TypeScript comes from apps/desktop's node_modules unless TYPESCRIPT_PATH points elsewhere.

const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawnSync } = require('child_process');

const pkgDir = path.resolve(__dirname, '..');
const tsPath =
  process.env.TYPESCRIPT_PATH ||
  require.resolve('typescript', { paths: [path.resolve(pkgDir, '../../apps/desktop')] });
const ts = require(tsPath);

const outDir = fs.mkdtempSync(path.join(os.tmpdir(), 'email-dark-test-'));
const tests = [];

for (const dir of ['src', 'test']) {
  for (const name of fs.readdirSync(path.join(pkgDir, dir))) {
    if (!name.endsWith('.ts')) continue;
    const source = fs.readFileSync(path.join(pkgDir, dir, name), 'utf8');
    const { outputText } = ts.transpileModule(source, {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
        esModuleInterop: true,
      },
      fileName: name,
    });
    const out = path.join(outDir, dir, name.replace(/\.ts$/, '.js'));
    fs.mkdirSync(path.dirname(out), { recursive: true });
    fs.writeFileSync(out, outputText);
    if (dir === 'test') tests.push(out);
  }
}

const result = spawnSync(process.execPath, ['--test', ...tests], { stdio: 'inherit' });
fs.rmSync(outDir, { recursive: true, force: true });
process.exit(result.status === null ? 1 : result.status);
