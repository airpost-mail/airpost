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
const appDir = path.resolve(buildDir, '..');
const outputDir = path.join(appDir, 'dist');
const tmpdir = path.resolve(os.tmpdir(), 'airpost-build');
const packageJSON = require(path.join(appDir, 'package.json'));
const { compilerOptions } = require(path.join(appDir, 'tsconfig.json'));

const macBundleId = 'com.airpost.airpost';

const appCopyright =
  `Copyright (C) 2014-${new Date().getFullYear()} Foundry 376, LLC and Airpost contributors. ` +
  'Licensed under GPL-3.0-only.';

// entitlements.plist carries a AIRPOST_APPLE_TEAM_ID placeholder; signing needs the real team ID,
// which must match the provisioning profile.
function macMainEntitlementsPath() {
  const template = path.join(buildDir, 'resources', 'mac', 'entitlements.plist');
  const teamId = process.env.APPLE_TEAM_ID;
  if (!teamId) {
    throw new Error('SIGN_BUILD needs APPLE_TEAM_ID to fill in entitlements.plist');
  }
  const out = path.join(os.tmpdir(), 'airpost-entitlements.plist');
  fs.writeFileSync(out, fs.readFileSync(template, 'utf8').replace(/AIRPOST_APPLE_TEAM_ID/g, teamId));
  return out;
}

const sourceGlobs = [
  '{plugins,themes}/**/*.ts',
  '{plugins,themes}/**/*.tsx',
  '{plugins,themes}/**/*.jsx',
  'src/**/*.ts',
  'src/**/*.tsx',
  'src/**/*.jsx',
  '!src/**/node_modules/**/*.ts',
  '!src/**/node_modules/**/*.tsx',
  '!src/**/node_modules/**/*.jsx',
  '!{plugins,themes}/**/node_modules/**/*.ts',
  '!{plugins,themes}/**/node_modules/**/*.tsx',
  '!{plugins,themes}/**/node_modules/**/*.jsx',
  // Workspace packages from the repo's packages/ dir, linked in as TypeScript source.
  'node_modules/@airpost/*/src/**/*.ts',
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

/*
Workspace packages (packages/*) are linked in as node_modules/@airpost/<name>. Only their package.json
and the entries their `files` lists ship: their own node_modules holds their dev tooling (vitest,
typescript, json-schema-to-typescript: about 82 MB for packages/client) and never ships. Node
finds a package's runtime dependencies in the desktop's node_modules instead, so each one must
also be a desktop dependency.
*/
const workspaceScope = '@airpost';

function isWorkspacePackage(relativePackageDir) {
  return path.dirname(relativePackageDir) === path.join('node_modules', workspaceScope);
}

function workspacePackageEntries(packageDir) {
  const pkg = JSON.parse(fs.readFileSync(path.join(packageDir, 'package.json'), 'utf8'));
  if (!Array.isArray(pkg.files) || pkg.files.length === 0) {
    throw new Error(
      `${pkg.name} has no "files" in its package.json: list what it needs at runtime`
    );
  }
  pkg.files.forEach((entry) => {
    if (/[*?{}[\]]|^[.]|[/\\]/.test(entry) || !fs.existsSync(path.join(packageDir, entry))) {
      throw new Error(`${pkg.name}'s "files" entry ${entry} must be a top-level file or directory`);
    }
  });
  const desktopDeps = { ...packageJSON.dependencies, ...packageJSON.optionalDependencies };
  const missing = Object.keys({ ...pkg.dependencies, ...pkg.optionalDependencies }).filter(
    (dep) => !desktopDeps[dep]
  );
  if (missing.length) {
    throw new Error(
      `${pkg.name} depends on ${missing.join(', ')} at runtime: add them to apps/desktop's ` +
        `dependencies too, as the package's own node_modules doesn't ship`
    );
  }
  return ['package.json', ...pkg.files];
}

function resolveRealSymlinkPaths() {
  console.log('---> Resolving symlinks');
  const dirs = ['plugins', 'themes', 'src', 'tests', 'node_modules'];
  dirs.forEach((dir) => {
    const absoluteDir = path.join(appDir, dir);
    const entries = fs.readdirSync(absoluteDir).flatMap((name) => {
      const scopeDir = path.join(absoluteDir, name);
      // Scoped packages (`@airpost/email-dark`) are links inside a real `@scope` directory.
      const isScope = dir === 'node_modules' && name.startsWith('@');
      if (isScope && !fs.lstatSync(scopeDir).isSymbolicLink()) {
        return fs.readdirSync(scopeDir).map((child) => path.join(name, child));
      }
      return [name];
    });
    entries.forEach((packageName) => {
      const relativePackageDir = path.join(dir, packageName);
      const absolutePackageDir = path.join(absoluteDir, packageName);
      const realPackagePath = fs.realpathSync(absolutePackageDir);
      // Compare against the real parent, so a checkout under a symlinked dir (/tmp on macOS)
      // doesn't count every entry as a link.
      if (realPackagePath !== path.join(fs.realpathSync(absoluteDir), packageName)) {
        console.log(`  ---> Resolving '${relativePackageDir}' to '${realPackagePath}'`);
        const entries = isWorkspacePackage(relativePackageDir)
          ? workspacePackageEntries(realPackagePath)
          : null;
        symlinkedPackages.push({ realPackagePath, relativePackageDir, entries });
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
  symlinkedPackages.forEach(({ realPackagePath, relativePackageDir, entries }) => {
    const packagePath = path.join(buildPath, relativePackageDir);
    fsPlus.removeSync(packagePath);
    if (entries) {
      console.log(`  ---> Copying ${entries.join(', ')} from ${realPackagePath} to ${packagePath}`);
      entries.forEach((entry) =>
        fsExtra.copySync(path.join(realPackagePath, entry), path.join(packagePath, entry))
      );
    } else {
      console.log(`  ---> Copying ${realPackagePath} to ${packagePath}`);
      fsPlus.copySync(realPackagePath, packagePath);
    }
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
      if (/node_modules/.test(tsPath) && !/node_modules[/\\]@airpost[/\\]/.test(tsPath)) return;
      if (tsPath.endsWith('.d.ts')) return;
      const outPath = tsPath.replace(path.extname(tsPath), '.js');
      console.log(`  ---> Compiling ${path.relative(buildPath, tsPath)}`);
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

// Top-level entries of the desktop dir that ship. Anything else there is tooling. A new top-level
// runtime dir must be added here, or it silently won't ship.
const shippedTopLevel = [
  'package.json',
  'tsconfig.json', // compile-cache-ts.js reads its compilerOptions
  'default-profile',
  'keymaps',
  'lang',
  'menus',
  'node_modules',
  'plugins',
  'src',
  'static',
  'themes',
  'vendor',
  // Placed by apps/engine/packaging/build-release.sh --install-desktop.
  'airpost-engine',
  'airpost-engine.exe',
  'airpost-engine.THIRD-PARTY-NOTICES.txt',
  'airpost-mcp',
  'airpost-mcp.exe',
  'airpost-mcp.THIRD-PARTY-NOTICES.txt',
  'airpost',
  'airpost.exe',
  'airpost-gateway',
  'airpost-gateway.exe',
  'airpost-gateway.THIRD-PARTY-NOTICES.txt',
  // The web client airpost-gateway serves (`npm run build:desktop` in apps/web).
  'airpost-web',
];
const topLevelAllowlist = new RegExp(
  `^/(?!(?:${shippedTopLevel.map((n) => n.replace(/[.]/g, '\\.')).join('|')})(?:/|$))`
);

/*
Electron fuses, flipped in the extracted Electron binary before the app is copied in and signed.
RunAsNode, NODE_OPTIONS and the --inspect arguments are off, so signed Airpost can't be run as a Node
interpreter that borrows its keychain items, TCC grants and entitlements. Nothing in the app
runs itself with ELECTRON_RUN_AS_NODE: the engine, airpost, airpost-mcp and airpost-gateway are separate
binaries, and the database agent is a utility process (src/main/ipc/database-agent.ts).
Cookies are encrypted at rest. The app loads from app.asar only, and on macOS and Windows its
header is checked against the hash the packager embeds (ElectronAsarIntegrity in Info.plist, a
resource in the exe); Electron has no asar integrity check on Linux, and unpacked files are
never covered. GrantFileProtocolExtraPrivileges stays on: the windows run file:// pages.
*/
async function runFlipFuses({ buildPath, platform: target }) {
  const { flipFuses, FuseVersion, FuseV1Options } = require('@electron/fuses');
  const electron = {
    darwin: path.join(buildPath, 'Electron.app'),
    win32: path.join(buildPath, 'electron.exe'),
    linux: path.join(buildPath, 'electron'),
  }[target];
  // Editing the binary voids its signature. It is not re-signed here: this is still
  // Electron.app, and a signature made now would carry com.github.Electron as its identifier.
  // runAdHocSign (unsigned builds) or osxSign signs the finished Airpost.app instead.
  await flipFuses(electron, {
    version: FuseVersion.V1,
    [FuseV1Options.RunAsNode]: false,
    [FuseV1Options.EnableNodeOptionsEnvironmentVariable]: false,
    [FuseV1Options.EnableNodeCliInspectArguments]: false,
    [FuseV1Options.EnableCookieEncryption]: true,
    [FuseV1Options.OnlyLoadAppFromAsar]: true,
    [FuseV1Options.EnableEmbeddedAsarIntegrityValidation]: target !== 'linux',
  });
  console.log(`---> Flipped the Electron fuses in ${electron}`);
}

function buildPackagerOptions() {
  // See: https://github.com/electron-userland/electron-packager/blob/master/usage.txt
  return {
    appVersion: packageJSON.version,
    platform,
    protocols: [
      { name: 'Airpost Protocol', schemes: ['airpost'] },
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
      darwin: path.join(buildDir, 'resources', 'mac', 'airpost.icns'),
      win32: path.join(buildDir, 'resources', 'win', 'airpost.ico'),
      linux: undefined,
    }[platform],
    name: { darwin: 'Airpost', win32: 'Airpost', linux: 'airpost' }[platform],
    appCopyright,
    derefSymlinks: false,
    asar: {
      unpack:
        '{' +
        [
          // The Rust engine, the MCP server, the CLI and the gateway, placed by
          // apps/engine/packaging/build-release.sh --install-desktop. Main and Preferences
          // run them, and MCP clients and service units run binaries from here, so they must
          // be real files. airpost-gateway serves `airpost-web` from disk, so it is unpacked too.
          'airpost-engine',
          'airpost-engine.exe',
          'airpost-mcp',
          'airpost-mcp.exe',
          'airpost',
          'airpost.exe',
          'airpost-gateway',
          'airpost-gateway.exe',
          '**/airpost-web/**',
          '*.so',
          '*.so.*',
          '*.dll',
          '*.pdb',
          '*.node',
          // Windows tray icons: Electron loads .ico via LoadImage, which needs a real
          // file on disk rather than a copy extracted from the asar on every change.
          '*.ico',
          '**/vendor/**',
          '**/src/mail/quick-preview/**',
          '**/static/all_licenses.html',
          '**/static/extensions/**',
          '**/node_modules/spellchecker/**',
        ].join(',') +
        '}',
    },
    ignore: [
      // The desktop dir also holds dev and build tooling (build/, scripts/, docs, dotfiles) and
      // prune only removes devDependencies from node_modules, so ship listed top-level entries only.
      topLevelAllowlist,
      // Package tests only run in dev.
      /^\/(plugins|themes)\/[^/]+\/(specs|tests)(\/|$)/,
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
            // airpost-engine, airpost-mcp, chrome_crashpad_handler, ShipIt, etc.) must be signed
            // with only the basic com.apple.security.* entitlements — amfid
            // will reject any helper that carries restricted entitlements it
            // cannot match to a profile scoped to that binary.
            // Note: electron-osx-sign passes the .app bundle path (not the
            // inner executable path) when signing the top-level app bundle.
            const isMainExecutable = filePath.endsWith('/Airpost.app');
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
      CompanyName: 'Airpost',
      FileDescription: 'Airpost',
      LegalCopyright: appCopyright,
      ProductName: 'Airpost',
    },
    // NOTE: The following plist keys can NOT be set in the extra.plist since
    // they are manually overridden by electron-packager based on this config:
    //   CFBundleDisplayName, CFBundleExecutable, CFBundleIdentifier, CFBundleName
    // See https://github.com/electron-userland/electron-packager/blob/master/mac.js#L50
    extendInfo: path.join(buildDir, 'resources', 'mac', 'extra.plist'),
    extraResource:
      platform === 'darwin' ? [path.join(buildDir, 'resources', 'mac', 'new-mail.mp3')] : undefined,
    appBundleId: macBundleId,
    afterExtract: [runFlipFuses],
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
  // The packager copies a symlinked node_modules as the link (derefSymlinks is off), so the
  // asar would ship no modules, and runCopySymlinkedPackages would write the workspace
  // packages through the link into the node_modules it points at.
  if (fs.lstatSync(path.join(appDir, 'node_modules')).isSymbolicLink()) {
    throw new Error('apps/desktop/node_modules is a symlink: run `npm ci` in apps/desktop first');
  }
  const opts = buildPackagerOptions();
  console.log('---> Running packager with options:');
  console.log(util.inspect(opts, true, 7, true));

  const start = Date.now();
  const ongoing = setInterval(() => {
    const elapsed = Math.round((Date.now() - start) / 1000.0);
    console.log(`---> Packaging for ${elapsed}s`);
  }, 1000);

  resolveRealSymlinkPaths();

  let appPaths;
  try {
    appPaths = await packager(opts);
    console.log(`---> Done Successfully. Built into: ${appPaths}`);
  } finally {
    clearInterval(ongoing);
  }
  appPaths.forEach(checkWorkspacePackagesInAsar);
  if (platform === 'darwin') {
    for (const appPath of appPaths) {
      const app = path.join(appPath, 'Airpost.app');
      if (!process.env.SIGN_BUILD) await runAdHocSign(app);
      await checkSignatureIdentifier(app);
    }
  }
}

// Fails the build if the asar carries more of a workspace package than its package.json and
// `files`, such as its node_modules of dev dependencies.
function checkWorkspacePackagesInAsar(appPath) {
  // The asar module the packager wrote it with, not whichever version npm hoisted.
  const asar = require(
    require.resolve('@electron/asar', {
      paths: [path.dirname(require.resolve('@electron/packager'))],
    })
  );
  const resources =
    platform === 'darwin'
      ? path.join(appPath, 'Airpost.app', 'Contents', 'Resources')
      : path.join(appPath, 'resources');
  const asarPath = path.join(resources, 'app.asar');
  const scopePrefix = `/node_modules/${workspaceScope}/`;
  const allowed = {};
  const stray = asar
    .listPackage(asarPath, { isPack: false })
    .map((entry) => entry.split(path.sep).join('/'))
    .filter((entry) => {
      if (!entry.startsWith(scopePrefix)) return false;
      const [name, top] = entry.slice(scopePrefix.length).split('/');
      if (top === undefined) return false;
      if (!allowed[name]) {
        allowed[name] = workspacePackageEntries(
          path.join(appDir, 'node_modules', workspaceScope, name)
        );
      }
      return top === 'node_modules' || !allowed[name].includes(top);
    });
  if (stray.length) {
    const shown = stray.slice(0, 20).join('\n  ');
    throw new Error(
      `${asarPath} carries ${stray.length} files of workspace packages outside their package.json ` +
        `and "files":\n  ${shown}${stray.length > 20 ? '\n  ...' : ''}`
    );
  }
  console.log(`---> ${asarPath} carries only the workspace packages' runtime files`);
}

/*
Signs an unsigned build ad hoc, which arm64 needs to launch it at all. codesign takes each
bundle's identifier from its Info.plist (com.airpost.airpost, the helpers' .helper ids), so the
keychain and TCC see Airpost rather than Electron. Electron's designated requirement names its own
identifier and team, so it isn't kept.
*/
async function runAdHocSign(app) {
  await spawn({
    cmd: 'codesign',
    args: [
      '--sign',
      '-',
      '--force',
      '--deep',
      '--preserve-metadata=entitlements,flags,runtime',
      app,
    ],
  });
  console.log(`---> Signed ${app} ad hoc`);
}

async function checkSignatureIdentifier(app) {
  const { stderr } = await spawn({ cmd: 'codesign', args: ['-dv', app] });
  const match = /^Identifier=(.*)$/m.exec(stderr);
  const identifier = match ? match[1] : null;
  if (identifier !== macBundleId) {
    throw new Error(`${app} is signed as ${identifier}, not ${macBundleId}`);
  }
}

async function createMacZip() {
  const zipPath = path.join(outputDir, 'Airpost.zip');
  if (fs.existsSync(zipPath)) {
    fs.unlinkSync(zipPath);
  }
  const arch = process.env.OVERRIDE_TO_INTEL ? 'x64' : process.arch;
  const cwd = path.join(outputDir, `Airpost-darwin-${arch}`);
  await spawn({
    cmd: 'zip',
    args: ['-9', '-y', '-r', '-9', '-X', zipPath, 'Airpost.app'],
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

  const contentsDir = path.join(outputDir, `airpost-linux-${process.arch}`);
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
    linuxShareDir: '/usr/share/airpost',
    arch: linuxArch,
    section: 'mail',
    maintainer: 'Airpost contributors <https://github.com/airpost-mail/airpost/issues>',
    installedSize,
  };
  writeFromTemplate(path.join(linuxAssetsDir, 'debian', 'control.in'), data);
  writeFromTemplate(path.join(linuxAssetsDir, 'com.airpost.airpost.desktop.in'), data);
  writeFromTemplate(path.join(linuxAssetsDir, 'com.airpost.airpost.metainfo.xml.in'), data);

  const icon = path.join(linuxAssetsDir, 'icons', 'airpost-512.png');
  await spawn({
    cmd: path.join(buildDir, 'mkdeb'),
    args: [packageJSON.version, linuxArch, icon, linuxAssetsDir, contentsDir, outputDir],
  });
  console.log(`Created ${outputDir}/airpost-${packageJSON.version}-${linuxArch}.deb`);
}

async function createRpmInstaller() {
  if (!linuxArch) throw new Error(`Unsupported arch ${process.arch}`);

  const contentsDir = path.join(outputDir, `airpost-linux-${process.arch}`);
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
    linuxShareDir: '/usr/local/share/airpost',
    linuxAssetsDir,
    contentsDir,
  };

  writeFromTemplate(path.join(linuxAssetsDir, 'redhat', 'airpost.spec.in'), templateData);
  writeFromTemplate(path.join(linuxAssetsDir, 'com.airpost.airpost.desktop.in'), templateData);
  writeFromTemplate(path.join(linuxAssetsDir, 'com.airpost.airpost.metainfo.xml.in'), templateData);

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
