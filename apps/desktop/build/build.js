/* eslint global-require: 0 */
/* eslint import/no-dynamic-require: 0 */
/* eslint prefer-template: 0 */
/* eslint quote-props: 0 */
const fs = require('fs');
const path = require('path');
const os = require('os');
const util = require('util');
const { execSync, spawn: childSpawn } = require('child_process');
const fsExtra = require('fs-extra');
const fsPlus = require('fs-plus');
const glob = require('glob');
const _ = require('underscore');
const TypeScript = require('typescript');
const { packager } = require('@electron/packager');
const { writeLicenses } = require('./generate-licenses');

const platform = process.platform;
const buildDir = __dirname;
const appDir = path.resolve(buildDir, '..', 'app');
const outputDir = path.join(appDir, 'dist');
const tmpdir = path.resolve(os.tmpdir(), 'tern-build');
const packageJSON = require(path.join(appDir, 'package.json'));
const { compilerOptions } = require(path.join(appDir, 'tsconfig.json'));

const appCopyright =
  `Copyright (C) 2014-${new Date().getFullYear()} Foundry 376, LLC and Tern contributors. ` +
  'Licensed under GPL-3.0-only.';

// entitlements.plist carries a TERN_APPLE_TEAM_ID placeholder; signing needs the real team ID,
// which must match the provisioning profile.
function macMainEntitlementsPath() {
  const template = path.join(buildDir, 'resources', 'mac', 'entitlements.plist');
  const teamId = process.env.APPLE_TEAM_ID;
  if (!teamId) {
    throw new Error('SIGN_BUILD needs APPLE_TEAM_ID to fill in entitlements.plist');
  }
  const out = path.join(os.tmpdir(), 'tern-entitlements.plist');
  fs.writeFileSync(out, fs.readFileSync(template, 'utf8').replace(/TERN_APPLE_TEAM_ID/g, teamId));
  return out;
}

const sourceGlobs = [
  'internal_packages/**/*.ts',
  'internal_packages/**/*.tsx',
  'internal_packages/**/*.jsx',
  'src/**/*.ts',
  'src/**/*.tsx',
  'src/**/*.jsx',
  '!src/**/node_modules/**/*.ts',
  '!src/**/node_modules/**/*.tsx',
  '!src/**/node_modules/**/*.jsx',
  '!internal_packages/**/node_modules/**/*.ts',
  '!internal_packages/**/node_modules/**/*.tsx',
  '!internal_packages/**/node_modules/**/*.jsx',
  // Workspace packages from the repo's packages/ dir, linked in as TypeScript source.
  'node_modules/@tern/*/src/**/*.ts',
];

function spawn(options) {
  return new Promise((resolve, reject) => {
    const stdout = [];
    const stderr = [];
    const proc = childSpawn(options.cmd, options.args, options.opts);
    proc.stdout.on('data', (data) => stdout.push(data.toString()));
    proc.stderr.on('data', (data) => stderr.push(data.toString()));
    proc.on('error', reject);
    proc.on('close', (exitCode) => {
      const result = { stdout: stdout.join(''), stderr: stderr.join(''), code: exitCode };
      if (exitCode !== 0) {
        console.error(result.stderr);
        return reject(new Error(`${options.cmd} exited with code ${exitCode}`));
      }
      resolve(result);
    });
  });
}

const symlinkedPackages = [];

function resolveRealSymlinkPaths() {
  console.log('---> Resolving symlinks');
  const dirs = ['internal_packages', 'src', 'spec', 'node_modules'];
  dirs.forEach((dir) => {
    const absoluteDir = path.join(appDir, dir);
    const entries = fs.readdirSync(absoluteDir).flatMap((name) => {
      const scopeDir = path.join(absoluteDir, name);
      // Scoped packages (`@tern/email-dark`) are links inside a real `@scope` directory.
      const isScope = dir === 'node_modules' && name.startsWith('@');
      if (isScope && !fs.lstatSync(scopeDir).isSymbolicLink()) {
        return fs.readdirSync(scopeDir).map((child) => path.join(name, child));
      }
      return [name];
    });
    entries.forEach((packageName) => {
      const relativePackageDir = path.join(dir, packageName);
      const absolutePackageDir = path.join(absoluteDir, packageName);
      const realPackagePath = fs.realpathSync(absolutePackageDir).replace('/private/', '/');
      if (realPackagePath !== absolutePackageDir) {
        console.log(`  ---> Resolving '${relativePackageDir}' to '${realPackagePath}'`);
        symlinkedPackages.push({ realPackagePath, relativePackageDir });
      }
    });
  });
}

function runCopyPlatformSpecificResources({ buildPath, platform }) {
  const winResourcesSource = path.join(buildDir, 'resources', 'win');
  const winResourcesTarget = path.resolve(buildPath, '..');
  if (platform === 'win32') {
    fsPlus.copySync(winResourcesSource, winResourcesTarget);
  }
}

function runWriteCommitHashIntoPackage({ buildPath }) {
  const commit = execSync('git rev-parse HEAD').toString();
  const jsonPath = path.resolve(buildPath, 'package.json');
  let jsonString = fs.readFileSync(jsonPath).toString();
  jsonString = jsonString.replace('COMMIT_INSERTED_DURING_PACKAGING', commit.substr(0, 8));
  fs.writeFileSync(jsonPath, jsonString);
}

// For Electron versions that support the setuid sandbox on Linux, the
// chrome-sandbox helper executable must have the setuid (`+s` / `0o4000`) bit.
// See: https://github.com/electron/electron/pull/17269#issuecomment-470671914
function runUpdateSandboxHelperPermissions({ buildPath }) {
  const helperPath = path.resolve(buildPath, '..', '..', 'chrome-sandbox');
  if (fs.existsSync(helperPath)) {
    console.log('---> Changing chrome-sandbox permissions');
    fs.chmodSync(helperPath, 0o4755);
  } else {
    console.log('---> Could not find chrome-sandbox to change permissions');
  }
}

function runCopySymlinkedPackages({ buildPath }) {
  console.log('---> Moving symlinked node modules / internal packages into build folder.');
  symlinkedPackages.forEach(({ realPackagePath, relativePackageDir }) => {
    const packagePath = path.join(buildPath, relativePackageDir);
    console.log(`  ---> Copying ${realPackagePath} to ${packagePath}`);
    fsPlus.removeSync(packagePath);
    fsPlus.copySync(realPackagePath, packagePath);
  });
}

function runWriteLicenses({ buildPath, platform }) {
  console.log('---> Writing licences and notices');
  writeLicenses({ buildPath, platform, commit: execSync('git rev-parse HEAD').toString().trim() });
}

function writeFileEnsureDir(filePath, contents) {
  fsExtra.mkdirpSync(path.dirname(filePath));
  fs.writeFileSync(filePath, contents);
}

function runTranspilers({ buildPath }) {
  console.log('---> Running TypeScript Compiler');
  sourceGlobs.forEach((pattern) => {
    glob.sync(pattern, { cwd: buildPath }).forEach((relPath) => {
      const tsPath = path.join(buildPath, relPath);
      const tsCode = fs.readFileSync(tsPath).toString();
      if (/\.js$/.test(tsPath)) return;
      if (/node_modules/.test(tsPath) && !/node_modules[/\\]@tern[/\\]/.test(tsPath)) return;
      if (tsPath.endsWith('.d.ts')) return;
      const outPath = tsPath.replace(path.extname(tsPath), '.js');
      console.log(`  ---> Compiling ${tsPath.slice(tsPath.indexOf('/app') + 4)}`);
      const res = TypeScript.transpileModule(tsCode, { compilerOptions, fileName: tsPath });
      writeFileEnsureDir(outPath, res.outputText);
      if (res.sourceMapText) {
        writeFileEnsureDir(outPath + '.map', res.sourceMapText);
      }
      fs.unlinkSync(tsPath);
    });
  });
}

// Packaged builds ship no source maps, including the ones runTranspilers just wrote.
function runRemoveSourceMaps({ buildPath }) {
  const mapFiles = glob.sync('**/*.js.map', { cwd: buildPath });
  mapFiles.forEach((relPath) => fs.unlinkSync(path.join(buildPath, relPath)));
  console.log(`---> Removed ${mapFiles.length} source map files`);
}

function buildPackagerOptions() {
  // See: https://github.com/electron-userland/electron-packager/blob/master/usage.txt
  return {
    appVersion: packageJSON.version,
    platform,
    protocols: [
      { name: 'Tern Protocol', schemes: ['tern'] },
      { name: 'Mailto Protocol', schemes: ['mailto'] },
    ],
    dir: appDir,
    appCategoryType: 'public.app-category.business',
    tmpdir,
    arch: {
      win32: 'x64',
      darwin: process.env.OVERRIDE_TO_INTEL ? 'x64' : process.arch,
      linux: process.arch,
    }[platform],
    icon: {
      darwin: path.join(buildDir, 'resources', 'mac', 'tern.icns'),
      win32: path.join(buildDir, 'resources', 'win', 'tern.ico'),
      linux: undefined,
    }[platform],
    name: { darwin: 'Tern', win32: 'Tern', linux: 'tern' }[platform],
    appCopyright,
    derefSymlinks: false,
    asar: {
      unpack:
        '{' +
        [
          // The Rust engine, the MCP server and the CLI, placed by
          // apps/engine/packaging/build-release.sh --install-desktop. Preferences runs
          // tern-mcp and tern, and MCP clients and service units run binaries from here, so
          // they must be real files.
          'mailsync-rs',
          'mailsync-rs.exe',
          'tern-mcp',
          'tern-mcp.exe',
          'tern',
          'tern.exe',
          '*.so',
          '*.so.*',
          '*.dll',
          '*.pdb',
          '*.node',
          // Windows tray icons: Electron loads .ico via LoadImage, which needs a real
          // file on disk rather than a copy extracted from the asar on every change.
          '*.ico',
          '**/vendor/**',
          'examples/**',
          '**/src/tasks/**',
          '**/src/quickpreview/**',
          '**/static/all_licenses.html',
          '**/static/extensions/**',
          '**/node_modules/spellchecker/**',
        ].join(',') +
        '}',
    },
    ignore: [
      // top level dirs we never want
      /^\/dist.*/,
      /^\/docs.*/,
      /^\/spec.*/,
      // The C++ engine (legacy/mailsync-cpp) is never packaged, even if a local build left one
      // in app/: its dependencies (OpenSSL 1.1 on Windows, StanfordCPPLib) are not
      // GPL-compatible or have no licence. Only mailsync-rs ships.
      /^\/mailsync(\.exe|\.bin|\.dSYM.*)?$/,
      // better-sqlite3's test fixture; postinstall deletes it, this catches a stale tree.
      /test_extension\.node$/,
      // general dirs we never want
      /[/]+gh-pages$/,
      /[/]+docs$/,
      /[/]+obj[/]+gen/,
      /[/]+\.deps$/,
      // File types we know we never want in the prod build. Licence and notice files stay,
      // whatever their extension: MIT, BSD and Apache-2.0 require them in every copy.
      /^(?!.*[/\\]((MIT|BSD|APACHE)[-._])?(LICEN[CS]E|COPYING|NOTICE)[^/\\]*$).*\.md$/i,
      /\.log$/i,
      /\.yml$/i,
      /\.gz/i,
      /\.zip/i,
      /\.h$/,
      /\.cc$/,
      /\.flow$/,
      /\.gyp/,
      /\.mk/,
      /\.dYSM$/,
      /\.map$/,
      // specific (large) module bits we know we don't need
      /node_modules[/]+less[/]+dist$/,
      /node_modules[/]+react[/]+dist$/,
      /node_modules[/].*[/]tests?$/,
      /node_modules[/].*[/]coverage$/,
      /node_modules[/].*[/]benchmark$/,
    ],
    out: outputDir,
    overwrite: true,
    prune: true,
    osxSign: process.env.SIGN_BUILD
      ? {
          platform: 'darwin',
          // The provisioning profile is embedded once into the app bundle at
          // Contents/embedded.provisionprofile. It must be set here at the
          // top level — it is not a per-file option.
          provisioningProfile: process.env.APPLE_PROVISIONING_PROFILE_PATH,
          // Name or SHA-1 of the signing certificate. Needed when a keychain holds several
          // "Developer ID Application" certs: signing with one the profile doesn't list makes
          // amfid reject the restricted entitlements and the app refuses to launch.
          identity: process.env.APPLE_SIGNING_IDENTITY,
          optionsForFile: (filePath) => {
            // Only the main app bundle gets the full entitlements plist,
            // which includes restricted entitlements (keychain-access-groups,
            // com.apple.developer.*) that are validated against the embedded
            // provisioning profile. All helper binaries (Electron helpers,
            // mailsync-rs, tern-mcp, chrome_crashpad_handler, ShipIt, etc.) must be signed
            // with only the basic com.apple.security.* entitlements — amfid
            // will reject any helper that carries restricted entitlements it
            // cannot match to a profile scoped to that binary.
            // Note: electron-osx-sign passes the .app bundle path (not the
            // inner executable path) when signing the top-level app bundle.
            const isMainExecutable = filePath.endsWith('/Tern.app');
            return {
              hardenedRuntime: true,
              entitlements: isMainExecutable
                ? macMainEntitlementsPath()
                : path.join(buildDir, 'resources', 'mac', 'entitlements.child.plist'),
            };
          },
        }
      : undefined,
    osxNotarize: process.env.APPLE_ID
      ? {
          appleId: process.env.APPLE_ID,
          appleIdPassword: process.env.APPLE_ID_PASSWORD,
          ascProvider: process.env.APPLE_ID_ASC_PROVIDER,
          teamId: process.env.APPLE_TEAM_ID,
        }
      : undefined,
    win32metadata: {
      CompanyName: 'Tern',
      FileDescription: 'Tern',
      LegalCopyright: appCopyright,
      ProductName: 'Tern',
    },
    // NOTE: The following plist keys can NOT be set in the extra.plist since
    // they are manually overridden by electron-packager based on this config:
    //   CFBundleDisplayName, CFBundleExecutable, CFBundleIdentifier, CFBundleName
    // See https://github.com/electron-userland/electron-packager/blob/master/mac.js#L50
    extendInfo: path.join(buildDir, 'resources', 'mac', 'extra.plist'),
    extraResource:
      platform === 'darwin' ? [path.join(buildDir, 'resources', 'mac', 'new-mail.mp3')] : undefined,
    appBundleId: 'com.ternmail.tern',
    afterCopy: [
      runCopyPlatformSpecificResources,
      runWriteCommitHashIntoPackage,
      runUpdateSandboxHelperPermissions,
      runCopySymlinkedPackages,
      runWriteLicenses,
      runTranspilers,
      runRemoveSourceMaps,
    ],
  };
}

async function runPackager() {
  const opts = buildPackagerOptions();
  console.log('---> Running packager with options:');
  console.log(util.inspect(opts, true, 7, true));

  const start = Date.now();
  const ongoing = setInterval(() => {
    const elapsed = Math.round((Date.now() - start) / 1000.0);
    console.log(`---> Packaging for ${elapsed}s`);
  }, 1000);

  resolveRealSymlinkPaths();

  try {
    const appPaths = await packager(opts);
    console.log(`---> Done Successfully. Built into: ${appPaths}`);
  } finally {
    clearInterval(ongoing);
  }
}

async function createMacZip() {
  const zipPath = path.join(outputDir, 'Tern.zip');
  if (fs.existsSync(zipPath)) {
    fs.unlinkSync(zipPath);
  }
  const arch = process.env.OVERRIDE_TO_INTEL ? 'x64' : process.arch;
  const cwd = path.join(outputDir, `Tern-darwin-${arch}`);
  await spawn({
    cmd: 'zip',
    args: ['-9', '-y', '-r', '-9', '-X', zipPath, 'Tern.app'],
    opts: { cwd },
  });
  console.log(`>> Created ${zipPath}`);
}

function writeFromTemplate(filePath, data) {
  const template = _.template(String(fs.readFileSync(filePath)));
  const finishedPath = path.join(outputDir, path.basename(filePath).replace('.in', ''));
  writeFileEnsureDir(finishedPath, template(data));
  return finishedPath;
}

const linuxArch = { ia32: 'i386', x64: 'amd64', arm64: 'arm64' }[process.arch];

async function createDebInstaller() {
  if (!linuxArch) throw new Error(`Unsupported arch ${process.arch}`);

  const contentsDir = path.join(outputDir, `tern-linux-${process.arch}`);
  const linuxAssetsDir = path.resolve(path.join(buildDir, 'resources', 'linux'));

  // `du` failures (e.g. permission errors) are non-fatal — fall back to a
  // 200MB estimate so the .deb still gets built. Matches the old behavior.
  let installedSize = '200000';
  try {
    const { stdout } = await spawn({ cmd: 'du', args: ['-sk', contentsDir] });
    installedSize = stdout.split(/\s+/).shift() || '200000';
  } catch (err) {
    console.warn(
      `---> du failed (${err.message}), defaulting installed size to ${installedSize}KB`
    );
  }

  const data = {
    version: packageJSON.version,
    name: packageJSON.name,
    description: packageJSON.description,
    productName: packageJSON.productName,
    desktopName: packageJSON.desktopName || `${packageJSON.name}.desktop`,
    linuxShareDir: '/usr/share/tern',
    arch: linuxArch,
    section: 'mail',
    maintainer: 'Tern contributors <https://github.com/ternmail/ternmail/issues>',
    installedSize,
  };
  writeFromTemplate(path.join(linuxAssetsDir, 'debian', 'control.in'), data);
  writeFromTemplate(path.join(linuxAssetsDir, 'com.ternmail.tern.desktop.in'), data);
  writeFromTemplate(path.join(linuxAssetsDir, 'com.ternmail.tern.metainfo.xml.in'), data);

  const icon = path.join(linuxAssetsDir, 'icons', 'tern-512.png');
  await spawn({
    cmd: path.join(buildDir, 'mkdeb'),
    args: [packageJSON.version, linuxArch, icon, linuxAssetsDir, contentsDir, outputDir],
  });
  console.log(`Created ${outputDir}/tern-${packageJSON.version}-${linuxArch}.deb`);
}

async function createRpmInstaller() {
  if (!linuxArch) throw new Error(`Unsupported arch ${process.arch}`);

  const contentsDir = path.join(outputDir, `tern-linux-${process.arch}`);
  const linuxAssetsDir = path.resolve(path.join(buildDir, 'resources', 'linux'));
  const rpmDir = path.join(outputDir, 'rpm');
  if (fs.existsSync(rpmDir)) {
    fsPlus.removeSync(rpmDir);
  }

  const templateData = {
    name: packageJSON.name,
    version: packageJSON.version,
    description: packageJSON.description,
    productName: packageJSON.productName,
    desktopName: packageJSON.desktopName || `${packageJSON.name}.desktop`,
    linuxShareDir: '/usr/local/share/tern',
    linuxAssetsDir,
    contentsDir,
  };

  writeFromTemplate(path.join(linuxAssetsDir, 'redhat', 'tern.spec.in'), templateData);
  writeFromTemplate(path.join(linuxAssetsDir, 'com.ternmail.tern.desktop.in'), templateData);
  writeFromTemplate(path.join(linuxAssetsDir, 'com.ternmail.tern.metainfo.xml.in'), templateData);

  await spawn({
    cmd: path.join(buildDir, 'mkrpm'),
    args: [outputDir, contentsDir, linuxAssetsDir],
  });
  console.log(`Created rpm package in ${rpmDir}`);
}

async function main() {
  await runPackager();

  if (process.argv.includes('--skip-installers')) {
    console.log('---> Skipping installer creation (--skip-installers flag detected)');
    return;
  }

  if (platform === 'darwin') {
    await createMacZip();
  } else if (platform === 'linux') {
    await createDebInstaller();
    await createRpmInstaller();
  }
  // win32: the electron-winstaller step is run separately by
  // build/create-signed-windows-installer.js because of path issues.
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
