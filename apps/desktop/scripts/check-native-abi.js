#!/usr/bin/env node
// Checks that better-sqlite3 was built for Electron's ABI rather than Node's. A bare
// `npm rebuild` rebuilds it for Node, and the app then fails only when it opens the database.
// Electron in node mode loads modules with the same ABI as the app's processes.
const path = require('path');
const { spawnSync } = require('child_process');

function checkNativeAbi(modulesPath) {
  const electron = require('electron');
  const sqlite = path.join(modulesPath, 'better-sqlite3');
  const probe = `
    const Database = require(${JSON.stringify(sqlite)});
    const db = new Database(':memory:');
    if (db.prepare('select 1 as x').get().x !== 1) throw new Error('select 1 failed');
    db.close();
  `;
  const result = spawnSync(electron, ['-e', probe], {
    env: Object.assign({}, process.env, { ELECTRON_RUN_AS_NODE: '1' }),
    encoding: 'utf8',
  });
  if (result.status !== 0) {
    return `better-sqlite3 in ${modulesPath} doesn't load under Electron ${
      require('electron/package.json').version
    }. Run \`npm ci\`, not \`npm rebuild\`.\n${result.error || result.stderr}`;
  }
  return null;
}

module.exports = { checkNativeAbi };

if (require.main === module) {
  const modulesPath = path.resolve(
    process.argv[2] || path.join(__dirname, '..', 'app', 'node_modules')
  );
  const failure = checkNativeAbi(modulesPath);
  if (failure) {
    console.error(failure);
    process.exit(1);
  }
  console.log(`better-sqlite3 in ${modulesPath} loads under Electron.`);
}
