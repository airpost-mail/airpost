#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-only
//
// Fails when an npm package that ships (anything in package-lock.json not marked dev-only) has
// a licence outside ALLOWED: the npm side of deny.toml's [licenses] allowlist, so that
// GPL-3.0-compatible-only holds for the desktop and web bundles as it does for the crates.
//
//   node .github/scripts/check-npm-licenses.mjs apps/desktop apps/web
//
// Run after `npm ci` in each directory: the licence is read from the installed package.json,
// since the lockfile leaves it out for many older packages.

import fs from 'node:fs';
import path from 'node:path';

// deny.toml's allow list, plus the GPL family (compatible with GPL-3.0-only when it allows v3)
// and permissive licences common on npm that the FSF lists as GPL-compatible. BlueOak-1.0.0 is
// a permissive OSI licence (glob, lru-cache and friends).
const ALLOWED = new Set([
  'MIT',
  'MIT-0',
  'Apache-2.0',
  'Apache-2.0 WITH LLVM-exception',
  'ISC',
  'BSD-2-Clause',
  'BSD-3-Clause',
  '0BSD',
  'MPL-2.0',
  'Zlib',
  'Unicode-3.0',
  'CDLA-Permissive-2.0',
  'BlueOak-1.0.0',
  'CC0-1.0',
  'Unlicense',
  'WTFPL',
  'Python-2.0',
  'GPL-3.0-only',
  'GPL-3.0-or-later',
  'LGPL-3.0-only',
  'LGPL-3.0-or-later',
  'LGPL-2.1-or-later',
]);

// Non-SPDX spellings found in package.json files.
const ALIASES = {
  'MIT/X11': 'MIT',
  MPL2: 'MPL-2.0',
  'Apache 2.0': 'Apache-2.0',
  'Apache License, Version 2.0': 'Apache-2.0',
};

// Packages whose package.json doesn't say, checked by hand. Keep the reason.
const REVIEWED = {
  // No licence field; upstream LICENSE is MIT (build/resources/licenses/npm/get-document.txt).
  'get-document': 'MIT',
  // Open Works License, a permissive licence (build/resources/licenses/npm/owl.txt).
  'html-attributes': 'OWL',
  'svg-attributes': 'OWL',
  // Declares "BSD"; its LICENSE file is BSD-2-Clause (mammoth's dependency).
  duck: 'BSD-2-Clause',
  // Fonts the web client bundles. OFL-1.1 covers the font files, which ship alongside the
  // GPL code as a separate work; the CSS is MIT.
  '@fontsource-variable/instrument-sans': 'OFL-1.1',
  '@fontsource-variable/jetbrains-mono': 'OFL-1.1',
};

function declared(pkg) {
  if (typeof pkg.license === 'string') return pkg.license;
  if (pkg.license && pkg.license.type) return pkg.license.type;
  if (Array.isArray(pkg.licenses)) {
    return pkg.licenses.map((l) => (typeof l === 'string' ? l : l.type)).join(' OR ');
  }
  return null;
}

function normalize(license) {
  const s = license.trim();
  if (ALIASES[s]) return ALIASES[s];
  // "MIT (http://mootools.net/license.txt)"
  const m = /^([A-Za-z0-9.+-]+) \(https?:\/\/[^)]*\)$/.exec(s);
  return m ? m[1] : s;
}

// Evaluates an SPDX expression (OR, AND, WITH, parentheses) against ALLOWED.
function allowed(expr) {
  const tokens = expr.replace(/\(/g, ' ( ').replace(/\)/g, ' ) ').split(/\s+/).filter(Boolean);
  let i = 0;
  const peek = () => tokens[i];
  function primary() {
    if (peek() === '(') {
      i++;
      const v = or();
      if (tokens[i++] !== ')') throw new Error('unbalanced');
      return v;
    }
    let id = tokens[i++];
    if (id === undefined) throw new Error('truncated');
    if (peek() === 'WITH') {
      i++;
      id = `${id} WITH ${tokens[i++]}`;
    }
    return ALLOWED.has(normalize(id)) || ALLOWED.has(id.replace(/\+$/, '-or-later'));
  }
  function and() {
    let v = primary();
    while (peek() === 'AND') {
      i++;
      v = primary() && v;
    }
    return v;
  }
  function or() {
    let v = and();
    while (peek() === 'OR') {
      i++;
      v = and() || v;
    }
    return v;
  }
  try {
    const v = or();
    return i === tokens.length && v;
  } catch {
    return false;
  }
}

let failures = 0;
for (const dir of process.argv.slice(2)) {
  const lock = JSON.parse(fs.readFileSync(path.join(dir, 'package-lock.json'), 'utf8'));
  let checked = 0;
  for (const [key, entry] of Object.entries(lock.packages || {})) {
    // The root, our own file: packages, and the dev-only tree (never bundled or packaged).
    if (key === '' || entry.link || entry.dev) continue;
    const name = entry.name || key.slice(key.lastIndexOf('node_modules/') + 'node_modules/'.length);
    let license = entry.license ?? null;
    const manifest = path.join(dir, key, 'package.json');
    if (fs.existsSync(manifest)) {
      license = declared(JSON.parse(fs.readFileSync(manifest, 'utf8'))) ?? license;
    } else if (!entry.optional) {
      console.error(`${dir}: ${key} is not installed; run npm ci first`);
      failures++;
      continue;
    } else if (license === null) {
      continue; // an optional package for another platform
    }
    checked++;
    if (REVIEWED[name]) continue;
    const ok = license !== null && allowed(normalize(license));
    if (!ok) {
      console.error(`${dir}: ${name}@${entry.version}: ${license ?? 'no licence declared'}`);
      failures++;
    }
  }
  console.log(`${dir}: ${checked} packages checked`);
}

if (failures) {
  console.error(
    `\n${failures} package(s) with a licence outside the GPL-3.0-compatible allowlist. Replace the ` +
      'dependency, or after checking the licence add it to REVIEWED in .github/scripts/check-npm-licenses.mjs.',
  );
  process.exit(1);
}
