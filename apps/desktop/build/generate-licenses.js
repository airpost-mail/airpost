/* eslint global-require: 0 */
/* eslint prefer-template: 0 */
// Writes static/all_licenses.html (Help > View Licenses) into a packaged app from what the
// package actually contains: Tern's licence and notice, the Rust engine's crate notices, the
// licence texts of every npm package left after pruning, every other licence file in the tree,
// and the notices for vendored files that carry no licence file of their own.
const fs = require('fs');
const path = require('path');

const repoRoot = path.resolve(__dirname, '..', '..', '..');
const appDir = path.resolve(__dirname, '..');
const licensesDir = path.join(__dirname, 'resources', 'licenses');

const LICENSE_FILE = /^((MIT|BSD|APACHE)[-._])?(LICEN[CS]E|COPYING|NOTICE|OFL)([-._][\w.-]*)?$/i;
const NOT_A_LICENSE = /\.(js|mjs|cjs|ts|json|html?|css|map|png|svg)$/i;

// Vendored code and assets without a licence file in the tree, or copied into our own sources.
const VENDORED = [
  {
    name: 'Twemoji graphics',
    where: 'static/images/composer-emoji/twitter/',
    license: 'CC-BY-4.0',
    file: 'twemoji.txt',
  },
  {
    name: 'Font Awesome 4.7.0',
    where: 'static/font-awesome.min.css, static/fonts/fontawesome-webfont.ttf',
    license: 'OFL-1.1 (font), MIT (CSS)',
    file: 'font-awesome.txt',
  },
  {
    name: 'DOMPurify 3.3.1',
    where: 'vendor/dompurify/',
    license: 'Apache-2.0 OR MPL-2.0',
    file: 'dompurify.txt',
  },
  {
    name: 'Prism 1.15.0',
    where: 'vendor/prism-1.15.0/',
    license: 'MIT',
    file: 'prism.txt',
  },
  {
    name: 'SheetJS Community Edition 0.18.5',
    where: 'vendor/xlsx-0.18.5/',
    license: 'Apache-2.0',
    file: 'sheetjs.txt',
  },
  {
    name: 'AngularJS (ngSanitize)',
    where: 'encodeHTMLEntities in src/flux/models/utils.ts',
    license: 'MIT',
    file: 'angularjs.txt',
  },
  {
    name: 'slate-html-serializer',
    where: 'TEXT_RULE_IMPROVED in src/components/composer-editor/conversion.tsx',
    license: 'MIT',
    file: 'slate-html-serializer.txt',
  },
];

function escapeHTML(text) {
  return String(text)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function read(file) {
  return fs.readFileSync(file, 'utf8');
}

function isLicenseFile(dir, name) {
  if (!LICENSE_FILE.test(name) || NOT_A_LICENSE.test(name)) return false;
  try {
    return fs.statSync(path.join(dir, name)).isFile();
  } catch (err) {
    return false;
  }
}

function licenseFilesIn(dir) {
  return fs
    .readdirSync(dir)
    .filter((name) => isLicenseFile(dir, name))
    .sort()
    .map((name) => path.join(dir, name));
}

// npm packages that ship no licence file, with the text we found upstream (resources/licenses/npm).
const NPM_LICENSE_TEXTS = {
  'get-document': 'get-document.txt',
  'html-attributes': 'owl.txt',
  'svg-attributes': 'owl.txt',
};

// Declared licences we can print the standard text for when a package ships no licence file.
const SPDX_ALIASES = [
  [/^MIT\b/i, 'MIT'],
  [/^ISC$/i, 'ISC'],
  [/^BSD-2-Clause$/i, 'BSD-2-Clause'],
  [/^(Apache-2\.0|Apache License, Version 2\.0)$/i, 'Apache-2.0'],
  [/^(MPL-2\.0|MPL2)$/i, 'MPL-2.0'],
];

function standardText(license) {
  const match = license && SPDX_ALIASES.find(([re]) => re.test(license.trim()));
  return match
    ? { id: match[1], text: read(path.join(licensesDir, 'spdx', `${match[1]}.txt`)) }
    : null;
}

function authorOf(pkg) {
  const author = pkg.author;
  if (typeof author === 'string') return author;
  if (author && author.name) return author.name;
  return `the ${pkg.name} authors`;
}

function declaredLicense(pkg) {
  if (typeof pkg.license === 'string') return pkg.license;
  if (pkg.license && pkg.license.type) return pkg.license.type;
  if (Array.isArray(pkg.licenses)) return pkg.licenses.map((l) => l.type || l).join(' OR ');
  return null;
}

// Every package under any node_modules directory in the tree, once per name@version.
function collectPackages(root) {
  const packages = new Map();
  const visitModulesDir = (modulesDir) => {
    for (const entry of fs.readdirSync(modulesDir)) {
      if (entry.startsWith('.')) continue;
      const entryPath = path.join(modulesDir, entry);
      if (!fs.lstatSync(entryPath).isDirectory()) continue;
      const pkgDirs = entry.startsWith('@')
        ? fs.readdirSync(entryPath).map((name) => path.join(entryPath, name))
        : [entryPath];
      for (const pkgDir of pkgDirs) visitPackage(pkgDir);
    }
  };
  const visitPackage = (pkgDir) => {
    const pkgJSON = path.join(pkgDir, 'package.json');
    if (!fs.existsSync(pkgJSON)) return;
    let pkg;
    try {
      pkg = JSON.parse(read(pkgJSON));
    } catch (err) {
      return;
    }
    if (pkg.name && pkg.version) {
      const key = `${pkg.name}@${pkg.version}`;
      if (!packages.has(key)) {
        packages.set(key, {
          key,
          name: pkg.name,
          license: declaredLicense(pkg),
          files: licenseFilesIn(pkgDir),
          author: authorOf(pkg),
        });
      }
    }
    const nested = path.join(pkgDir, 'node_modules');
    if (fs.existsSync(nested)) visitModulesDir(nested);
  };
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir)) {
      const entryPath = path.join(dir, entry);
      if (!fs.lstatSync(entryPath).isDirectory()) continue;
      if (entry === 'node_modules') visitModulesDir(entryPath);
      else walk(entryPath);
    }
  };
  walk(root);
  return [...packages.values()].sort((a, b) => a.key.localeCompare(b.key));
}

// Licence files outside node_modules (themes, pdf.js, fonts).
function collectTreeLicenseFiles(root) {
  const found = [];
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir)) {
      const entryPath = path.join(dir, entry);
      const stat = fs.lstatSync(entryPath);
      if (stat.isDirectory()) {
        if (entry !== 'node_modules') walk(entryPath);
      } else if (isLicenseFile(dir, entry)) {
        found.push(entryPath);
      }
    }
  };
  walk(root);
  return found.sort();
}

// The Electron distribution's LICENSE and LICENSES.chromium.html sit at the root of the
// packager's staging dir, a few levels above resources/app.
function findElectronLicenseDir(buildPath) {
  let dir = buildPath;
  for (let i = 0; i < 6; i++) {
    dir = path.dirname(dir);
    const both = ['LICENSE', 'LICENSES.chromium.html'];
    if (both.every((f) => fs.existsSync(path.join(dir, f)))) return dir;
  }
  return null;
}

function section(id, title, body) {
  return `<section id="${id}"><h2>${escapeHTML(title)}</h2>\n${body}</section>\n`;
}

function pre(text) {
  return `<pre>${escapeHTML(text.trim())}</pre>\n`;
}

// The repository in the root Cargo.toml's workspace.package, unless CI says otherwise.
const DEFAULT_REPOSITORY = 'https://github.com/ternmail/ternmail';

function sourceURLFor(commit) {
  const { TERN_SOURCE_URL, GITHUB_SERVER_URL, GITHUB_REPOSITORY } = process.env;
  if (TERN_SOURCE_URL) return TERN_SOURCE_URL;
  const repo =
    GITHUB_SERVER_URL && GITHUB_REPOSITORY
      ? `${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}`
      : DEFAULT_REPOSITORY;
  return `${repo}/tree/${commit}`;
}

/**
 * Writes the licence page into `buildPath/static/all_licenses.html` and puts LICENSE.md,
 * NOTICE.md and the engine's and tern-mcp's THIRD-PARTY-NOTICES in the resources directory,
 * outside the asar. On macOS it also copies Electron's LICENSE and LICENSES.chromium.html
 * into Contents/Resources, since the zip holds only the .app.
 *
 * Throws in CI (`CI` set) when the engine or tern-mcp is packaged without its crate notices,
 * Electron's licence files cannot be found, or an npm package has no licence text, so a
 * release cannot ship without them.
 */
function writeLicenses({ buildPath, platform, commit }) {
  const resourcesDir = path.resolve(buildPath, '..');
  const inCI = Boolean(process.env.CI);

  const ternLicense = read(path.join(repoRoot, 'LICENSE.md'));
  const ternNotice = read(path.join(repoRoot, 'NOTICE.md'));
  fs.copyFileSync(path.join(repoRoot, 'LICENSE.md'), path.join(resourcesDir, 'LICENSE.md'));
  fs.copyFileSync(path.join(repoRoot, 'NOTICE.md'), path.join(resourcesDir, 'NOTICE.md'));

  const hasEngine = ['mailsync-rs', 'mailsync-rs.exe'].some((f) =>
    fs.existsSync(path.join(buildPath, f))
  );
  const engineNoticesPath =
    process.env.TERN_ENGINE_NOTICES || path.join(appDir, 'mailsync-rs.THIRD-PARTY-NOTICES.txt');
  let engineNotices = null;
  if (fs.existsSync(engineNoticesPath)) {
    engineNotices = read(engineNoticesPath);
    fs.copyFileSync(engineNoticesPath, path.join(resourcesDir, 'THIRD-PARTY-NOTICES-engine.txt'));
  } else if (hasEngine) {
    const msg = `The Rust engine is packaged but ${engineNoticesPath} is missing (run apps/engine/packaging/build-release.sh --install-desktop with cargo-about installed)`;
    if (inCI) throw new Error(msg);
    console.warn(`---> ${msg}`);
  }

  const hasMcp = ['tern-mcp', 'tern-mcp.exe'].some((f) => fs.existsSync(path.join(buildPath, f)));
  const mcpNoticesPath =
    process.env.TERN_MCP_NOTICES || path.join(appDir, 'tern-mcp.THIRD-PARTY-NOTICES.txt');
  let mcpNotices = null;
  if (fs.existsSync(mcpNoticesPath)) {
    mcpNotices = read(mcpNoticesPath);
    fs.copyFileSync(mcpNoticesPath, path.join(resourcesDir, 'THIRD-PARTY-NOTICES-tern-mcp.txt'));
  } else if (hasMcp) {
    const msg = `tern-mcp is packaged but ${mcpNoticesPath} is missing (run apps/engine/packaging/build-release.sh --install-desktop with cargo-about installed)`;
    if (inCI) throw new Error(msg);
    console.warn(`---> ${msg}`);
  }

  const electronDir = findElectronLicenseDir(buildPath);
  let electronLicense = null;
  if (electronDir) {
    electronLicense = read(path.join(electronDir, 'LICENSE'));
    if (platform === 'darwin') {
      fs.copyFileSync(
        path.join(electronDir, 'LICENSE'),
        path.join(resourcesDir, 'LICENSE.electron')
      );
      fs.copyFileSync(
        path.join(electronDir, 'LICENSES.chromium.html'),
        path.join(resourcesDir, 'LICENSES.chromium.html')
      );
    }
  } else if (inCI) {
    throw new Error("Could not find Electron's LICENSES.chromium.html above " + buildPath);
  }

  const sourceURL = sourceURLFor(commit);
  const preamble = read(path.join(licensesDir, 'all_licenses_preamble.html'))
    .replace(/\{\{SOURCE_URL\}\}/g, escapeHTML(sourceURL))
    .replace(/\{\{COMMIT\}\}/g, escapeHTML(commit));

  const parts = [preamble];
  parts.push(section('tern-license', 'GNU General Public License, version 3', pre(ternLicense)));
  parts.push(section('tern-notice', 'Tern notice (NOTICE.md)', pre(ternNotice)));

  if (engineNotices) {
    parts.push(section('engine', 'Sync engine: Rust crates', pre(engineNotices)));
  }
  if (mcpNotices) {
    parts.push(section('tern-mcp', 'MCP server (tern-mcp): Rust crates', pre(mcpNotices)));
  }

  if (electronLicense) {
    const where =
      platform === 'darwin'
        ? 'Contents/Resources/LICENSES.chromium.html'
        : 'LICENSES.chromium.html in the application folder';
    parts.push(
      section(
        'electron',
        'Electron and Chromium',
        pre(electronLicense) +
          `<p>The licences of Chromium and the libraries it includes are in ${escapeHTML(where)}.</p>\n`
      )
    );
  }

  const vendored = VENDORED.map(
    (v) =>
      `<h3>${escapeHTML(v.name)}</h3>\n<p>${escapeHTML(v.where)}. Licence: ${escapeHTML(v.license)}.</p>\n` +
      pre(read(path.join(licensesDir, v.file)))
  ).join('');
  parts.push(section('vendored', 'Vendored code and assets', vendored));

  const treeFiles = collectTreeLicenseFiles(buildPath).filter(
    (f) => !['LICENSE.md', 'NOTICE.md'].includes(path.relative(buildPath, f))
  );
  parts.push(
    section(
      'tree',
      'Other bundled components',
      treeFiles
        .map((f) => `<h3>${escapeHTML(path.relative(buildPath, f))}</h3>\n` + pre(read(f)))
        .join('')
    )
  );

  const packages = collectPackages(buildPath);
  const unlicensed = [];
  const npm = packages
    .map((p) => {
      const head = `<h3 id="npm-${escapeHTML(p.key)}">${escapeHTML(p.key)}</h3>\n<p>Licence: ${escapeHTML(
        p.license || 'not declared'
      )}</p>\n`;
      if (p.files.length === 0 && NPM_LICENSE_TEXTS[p.name]) {
        return head + pre(read(path.join(licensesDir, 'npm', NPM_LICENSE_TEXTS[p.name])));
      }
      if (p.files.length === 0) {
        const standard = standardText(p.license);
        if (!standard) {
          unlicensed.push(p.key);
          return head + '<p>The package ships no licence file.</p>\n';
        }
        // MIT, ISC and BSD need the copyright line; package.json's author is the best record.
        return (
          head +
          `<p>The package ships no licence file; this is the standard ${standard.id} text.</p>\n` +
          pre(`Copyright (c) ${p.author}\n\n${standard.text}`)
        );
      }
      return (
        head +
        p.files
          .map((f) => `<p><code>${escapeHTML(path.basename(f))}</code></p>\n` + pre(read(f)))
          .join('')
      );
    })
    .join('');
  if (unlicensed.length && inCI) {
    throw new Error(
      `No licence text for ${unlicensed.join(', ')}: add one to NPM_LICENSE_TEXTS or drop the package`
    );
  }
  parts.push(section('npm', `npm packages (${packages.length})`, npm));

  const html = `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Licenses</title>
<style>
body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; margin: 2em auto; max-width: 60em; padding: 0 1em; line-height: 1.4; }
pre { white-space: pre-wrap; word-wrap: break-word; background: #f6f6f6; padding: 0.8em; font-size: 12px; }
h2 { border-top: 1px solid #ccc; padding-top: 1em; margin-top: 2em; }
h3 { font-size: 1em; margin-top: 1.5em; }
</style>
</head>
<body>
${parts.join('\n')}
</body>
</html>
`;
  const outPath = path.join(buildPath, 'static', 'all_licenses.html');
  fs.mkdirSync(path.dirname(outPath), { recursive: true });
  fs.writeFileSync(outPath, html);
  console.log(
    `---> Wrote ${outPath}: ${packages.length} npm packages, ${treeFiles.length} other licence files` +
      (unlicensed.length ? `; no licence file in ${unlicensed.join(', ')}` : '')
  );
}

module.exports = { writeLicenses };
