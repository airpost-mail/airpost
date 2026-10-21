#!/usr/bin/env node
/* eslint global-require: 0 */
/* eslint quote-props: 0 */
const path = require('path');
const fs = require('fs');
const { safeExec } = require('./child-process-wrapper.js');
const { checkNativeAbi } = require('./check-native-abi.js');

// One node_modules serves the dev tooling (run by Node) and the app (run by Electron). npm
// installs native modules for Node; the app's own native modules are rebuilt here for Electron.
const desktopDir = path.resolve(__dirname, '..');
const pkg = require('../package.json');
const modulesPath = path.join(desktopDir, 'node_modules');
const electronTarget = pkg.devDependencies.electron;
const electronEnv = Object.assign({}, process.env, {
  npm_config_target: electronTarget,
  npm_config_arch: process.env.OVERRIDE_TO_INTEL ? 'x64' : process.arch,
  npm_config_target_arch: process.env.OVERRIDE_TO_INTEL ? 'x64' : process.arch,
  npm_config_disturl: 'https://electronjs.org/headers',
  npm_config_runtime: 'electron',
});

function exec(cmd, options = {}) {
  return new Promise((resolve) => {
    safeExec(cmd, Object.assign({ cwd: desktopDir }, options), (err, stdout) => resolve(stdout));
  });
}

// Runtime deps (and optional ones) with a native build. Dev tools keep their Node build.
function nativeRuntimeModules() {
  const names = Object.keys(Object.assign({}, pkg.dependencies, pkg.optionalDependencies || {}));
  return names.filter((name) => fs.existsSync(path.join(modulesPath, name, 'binding.gyp')));
}

async function sqliteMissingNanosleep() {
  const sqliteLibDir = path.join(modulesPath, 'better-sqlite3', 'build', 'Release');
  const staticLib = path.join(sqliteLibDir, 'sqlite3.a');
  const sharedLib = path.join(sqliteLibDir, 'better_sqlite3.node');
  // Check the static lib first (build-from-source), then the prebuilt .node binary
  const target = fs.existsSync(staticLib) ? staticLib : sharedLib;
  const resp = await exec(`nm '${target}' | grep nanosleep`, { ignoreStderr: true });
  return resp === '';
}

async function run() {
  const native = nativeRuntimeModules();
  console.log(`\n-- Rebuilding ${native.join(', ')} for Electron ${electronTarget} --`);
  await exec(`npm rebuild ${native.join(' ')} --no-audit`, { env: electronEnv });

  // better-sqlite3 builds a SQLite extension for its own test suite. Nothing loads it, and the
  // packager would ship it next to the real binary.
  fs.rmSync(path.join(modulesPath, 'better-sqlite3', 'build', 'Release', 'test_extension.node'), {
    force: true,
  });

  // if SQlite was not built with HAVE_NANOSLEEP, do not ship this build! We need nanosleep
  // support so that multiple processes can connect to the sqlite file at the same time.
  // Without it, transactions only retry every 1 sec instead of every 10ms, leading to
  // awful db lock contention.  https://github.com/WiseLibs/better-sqlite3/issues/597
  if (['linux', 'darwin'].includes(process.platform) && (await sqliteMissingNanosleep())) {
    console.error(`better-sqlite compiled without -HAVE_NANOSLEEP, do not ship this build!`);
    process.exit(1001);
  }

  // The host's Electron can't load a module cross-built for another arch (OVERRIDE_TO_INTEL).
  const abiFailure = electronEnv.npm_config_arch === process.arch && checkNativeAbi(modulesPath);
  if (abiFailure) {
    console.error(abiFailure);
    process.exit(1);
  }

  // The Rust engine is built by apps/engine/packaging/build-release.sh --install-desktop.
  if (!fs.existsSync(path.join(desktopDir, 'tern-engine'))) {
    console.log(
      `\n-- No sync engine in apps/desktop. Build the Rust one with ` +
        `'apps/engine/packaging/build-release.sh --install-desktop' (installs tern-engine). --`
    );
  }
}

run();
