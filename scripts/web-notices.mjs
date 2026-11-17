// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors
//
// Writes the third-party notices for the built web client (apps/web/dist): the licence files of
// every npm package whose code the bundle contains, read from the build's source maps, plus the
// web client's direct dependencies (fonts and Tailwind's preflight reach the CSS without a
// source map entry).
//
//   node scripts/web-notices.mjs <apps/web> <dist> <output>
//
// Fails when a package's licence is outside the accepted list, or it ships no licence file and
// isn't MIT (whose text is then filled in from package.json's author).

import fs from 'node:fs';
import path from 'node:path';

const ACCEPTED = new Set([
  'MIT',
  'ISC',
  'Apache-2.0',
  'BSD-2-Clause',
  'BSD-3-Clause',
  '0BSD',
  'MPL-2.0',
  'OFL-1.1',
  'Unlicense',
  'CC0-1.0',
]);
const LICENSE_FILE = /^(LICEN[CS]E|COPYING|NOTICE)([-._].*)?$/i;
const MIT = `Permission is hereby granted, free of charge, to any person obtaining a copy of this software
and associated documentation files (the "Software"), to deal in the Software without
restriction, including without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.`;

const [webDir, distDir, out] = process.argv.slice(2);
if (!out) {
  console.error('usage: web-notices.mjs <apps/web> <dist> <output>');
  process.exit(2);
}

function walk(dir, files = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(p, files);
    else files.push(p);
  }
  return files;
}

// `../node_modules/@scope/name/dist/x.js` -> `node_modules/@scope/name`; the last
// node_modules segment wins, so nested copies count as their own package.
function packageRoot(source) {
  const parts = source.split('/');
  const i = parts.lastIndexOf('node_modules');
  if (i < 0 || i + 1 >= parts.length) return null;
  const n = parts[i + 1].startsWith('@') ? 2 : 1;
  return parts.slice(0, i + 1 + n).join('/');
}

const roots = new Set();
for (const file of walk(distDir).filter((f) => f.endsWith('.map'))) {
  const map = JSON.parse(fs.readFileSync(file, 'utf8'));
  for (const source of map.sources ?? []) {
    const root = packageRoot(path.relative(webDir, path.resolve(path.dirname(file), source)));
    if (root) roots.add(root);
  }
}
const manifest = JSON.parse(fs.readFileSync(path.join(webDir, 'package.json'), 'utf8'));
for (const name of Object.keys(manifest.dependencies ?? {})) roots.add(`node_modules/${name}`);

const problems = [];
const sections = [];
for (const root of [...roots].sort()) {
  const dir = path.join(webDir, root);
  const pkgFile = path.join(dir, 'package.json');
  if (!fs.existsSync(pkgFile)) {
    problems.push(`${root}: not installed (run npm ci in ${webDir})`);
    continue;
  }
  const pkg = JSON.parse(fs.readFileSync(pkgFile, 'utf8'));
  const license = typeof pkg.license === 'string' ? pkg.license : pkg.license?.type;
  // Some alternative of an `A OR (B AND C)` expression must be accepted in full.
  const alternatives = (license ?? '').replace(/[()]/g, '').split(/\s+OR\s+/);
  const ok = alternatives.some((a) => a.split(/\s+AND\s+/).every((id) => ACCEPTED.has(id)));
  if (!license || !ok) {
    problems.push(`${pkg.name}@${pkg.version}: licence ${license ?? 'unknown'} is not accepted`);
  }
  const files = fs
    .readdirSync(dir)
    .filter((f) => LICENSE_FILE.test(f) && fs.statSync(path.join(dir, f)).isFile())
    .sort();
  const author = typeof pkg.author === 'string' ? pkg.author : pkg.author?.name;
  let texts;
  if (files.length) {
    texts = files.map((f) => fs.readFileSync(path.join(dir, f), 'utf8').trim());
  } else if (license === 'MIT' && author) {
    texts = [`The package ships no licence file; package.json says MIT.\n\nCopyright (c) ${author}\n\n${MIT}`];
  } else {
    problems.push(`${pkg.name}@${pkg.version}: no licence file`);
    continue;
  }
  sections.push(`${pkg.name} ${pkg.version} (${license})\n\n${texts.join('\n\n')}\n`);
}

if (problems.length) {
  console.error(problems.map((p) => `web-notices.mjs: ${p}`).join('\n'));
  process.exit(1);
}

const rule = '='.repeat(80);
const header = `Third-party software in the Airpost web client

The web client is licensed GPL-3.0-only (LICENSE.md). Its bundle contains code and fonts from
the npm packages below, under their own licences. The Airpost logos in brand/ are trademarks and
not covered by the GPL (TRADEMARKS.md).
`;
fs.mkdirSync(path.dirname(path.resolve(out)), { recursive: true });
fs.writeFileSync(out, `${header}\n${sections.map((s) => `${rule}\n${s}`).join('\n')}`);
console.error(`---> ${out} (${sections.length} packages)`);
