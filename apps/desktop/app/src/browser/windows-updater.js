/*
 * "Squirrel will spawn your app with command line flags on first run, updates,]
 * and uninstalls."
 *
 * Read: https://github.com/electron-archive/grunt-electron-installer#handling-squirrel-events
 * Read: https://github.com/electron/electron/blob/master/docs/api/auto-updater.md#windows
 *
 * When Tern gets installed on a Windows machine it gets put in:
 * C:\Users\<USERNAME>\AppData\Local\Tern\app-x.x.x
 *
 * The `process.execPath` is:
 * C:\Users\<USERNAME>\AppData\Local\Tern\app-x.x.x\tern.exe
 *
 * We manually copy everything in build/resources/win into a 'resources' folder
 * located inside the main app directory. See runCopyPlatformSpecificResources
 * in build/build.js
 *
 * This means `__dirname` should be:
 * C:\Users\<USERNAME>\AppData\Local\Tern\app-x.x.x\resources
 *
 * We also expect Squirrel Windows to have a file called `tern.exe` at:
 * C:\Users\<USERNAME>\AppData\Local\Tern\tern.exe
 */
const ChildProcess = require('child_process');
const fs = require('fs');
const path = require('path');
const os = require('os');
const { shell } = require('electron');

// Windows identity, shared with main.js. The toast activator CLSID is Tern's own: reusing
// Mailspring's would point its COM registration at whichever app registered last.
const APP_USER_MODEL_ID = 'com.ternmail.tern';
const TOAST_ACTIVATOR_CLSID = '{F4C2EDE9-B070-46C4-98C2-6B1A53639196}';

// C:\Users\<USERNAME>\AppData\Local\Tern\app-x.x.x
const appFolder = path.resolve(process.execPath, '..');

// C:\Users\<USERNAME>\AppData\Local\Tern\
const rootAppDataFolder = path.resolve(appFolder, '..');

// C:\Users\<USERNAME>\AppData\Local\Tern\Update.exe
const updateDotExe = path.join(rootAppDataFolder, 'Update.exe');

// "tern.exe"
const exeName = path.basename(process.execPath);

// Copy the real environment onto a null-prototype object before spawning, so a
// child inherits only genuine own variables. A plain object would additionally
// expose anything present on Object.prototype, which a prototype-pollution bug
// elsewhere in the process could abuse. See GHSA-gjr7-3mj2-cr54.
function sanitizedEnv(base) {
  return Object.assign(Object.create(null), base || process.env);
}

// Spawn a command and invoke the callback when it completes with an error
// and the output from standard out.
function spawn(command, args, callback, options = {}) {
  let stdout = '';
  let spawnedProcess = null;

  try {
    spawnedProcess = ChildProcess.spawn(command, args, {
      ...options,
      env: sanitizedEnv(options.env),
    });
  } catch (error) {
    // Spawn can throw an error
    setTimeout(() => callback && callback(error, stdout), 0);
    return;
  }

  spawnedProcess.stdout.on('data', (data) => {
    stdout += data;
  });

  let error = null;
  spawnedProcess.on('error', (processError) => {
    error = error || processError;
  });

  spawnedProcess.on('close', (code, signal) => {
    if (code !== 0) {
      error = error || new Error(`Command failed: ${signal || code}`);
    }
    if (error) {
      error.code = error.code || code;
      error.stdout = error.stdout || stdout;
    }
    if (callback) {
      callback(error, stdout);
    }
  });
}

// Spawn a command in detached mode without waiting for completion.
// This is used for Squirrel hooks where we need to exit quickly to avoid
// hitting Squirrel's 15-second timeout.
// See: https://github.com/Squirrel/Squirrel.Windows/issues/501
function spawnDetached(command, args) {
  try {
    const child = ChildProcess.spawn(command, args, {
      detached: true,
      stdio: 'ignore',
      env: sanitizedEnv(),
    });
    child.unref();
  } catch (error) {
    console.error(`Failed to spawn detached process: ${command} ${args.join(' ')}`, error.message);
  }
}

// Spawn the Update.exe with the given arguments and invoke the callback when
// the command completes.
function spawnUpdate(args, callback, options = {}) {
  spawn(updateDotExe, args, callback, options);
}

function createRegistryEntries({ allowEscalation, registerDefaultIfPossible }, callback) {
  const escapeBackticks = (str) => str.replace(/\\/g, '\\\\');

  const isWindows7 = os.release().startsWith('6.1');
  const requiresLocalMachine = isWindows7;

  // On Windows 7, we must write to LOCAL_MACHINE and need escalated privileges.
  // Don't do it at install time - wait for the user to ask Tern to be the default.
  if (requiresLocalMachine && !allowEscalation) {
    callback();
    return;
  }

  let regPath = 'reg.exe';
  if (process.env.SystemRoot) {
    regPath = path.join(process.env.SystemRoot, 'System32', 'reg.exe');
  }

  let spawnPath = regPath;
  let spawnArgs = [];
  if (requiresLocalMachine) {
    spawnPath = path.join(appFolder, 'resources', 'elevate.cmd');
    spawnArgs = [regPath];
  }

  fs.readFile(path.join(appFolder, 'resources', 'tern-mailto-registration.reg'), (err, data) => {
    if (err || !data) {
      callback(err);
      return;
    }
    const importTemplate = data.toString();
    let importContents = importTemplate.replace(
      /{{PATH_TO_ROOT_FOLDER}}/g,
      escapeBackticks(rootAppDataFolder)
    );
    importContents = importContents.replace(/{{PATH_TO_APP_FOLDER}}/g, escapeBackticks(appFolder));
    if (requiresLocalMachine) {
      importContents = importContents.replace(/{{HKEY_ROOT}}/g, 'HKEY_LOCAL_MACHINE');
    } else {
      importContents = importContents.replace(/{{HKEY_ROOT}}/g, 'HKEY_CURRENT_USER');
    }

    const importTempPath = path.join(os.tmpdir(), `tern-reg-${Date.now()}.reg`);

    fs.writeFile(importTempPath, importContents, (writeErr) => {
      if (writeErr) {
        callback(writeErr);
        return;
      }

      spawn(
        spawnPath,
        spawnArgs.concat(['import', escapeBackticks(importTempPath)]),
        (spawnErr) => {
          if (isWindows7 && registerDefaultIfPossible) {
            const defaultReg = path.join(appFolder, 'resources', 'tern-mailto-default.reg');
            spawn(
              spawnPath,
              spawnArgs.concat(['import', escapeBackticks(defaultReg)]),
              (spawnDefaultErr) => {
                callback(spawnDefaultErr, true);
              }
            );
          } else {
            callback(spawnErr, false);
          }
        }
      );
    });
  });
}

exports.spawn = spawnUpdate;
exports.createRegistryEntries = createRegistryEntries;

// Is the Update.exe installed with Tern?
exports.existsSync = () => fs.existsSync(updateDotExe);

// Register the AppUserModelId with a display name so Windows notifications
// show "Tern" instead of the raw AUMID
// Registry path: HKEY_CURRENT_USER\SOFTWARE\Classes\AppUserModelId\{AUMID}
function registerAppUserModelId(callback) {
  const aumid = APP_USER_MODEL_ID;
  const displayName = 'Tern';
  const iconPath = path.join(appFolder, 'resources', 'tern.ico');

  let regPath = 'reg.exe';
  if (process.env.SystemRoot) {
    regPath = path.join(process.env.SystemRoot, 'System32', 'reg.exe');
  }

  const regKey = `HKEY_CURRENT_USER\\SOFTWARE\\Classes\\AppUserModelId\\${aumid}`;

  // Add the DisplayName value
  spawn(
    regPath,
    ['add', regKey, '/v', 'DisplayName', '/t', 'REG_SZ', '/d', displayName, '/f'],
    (err) => {
      if (err) {
        console.warn('Failed to register AUMID DisplayName:', err);
      }
      // Also add IconUri if the icon exists
      if (fs.existsSync(iconPath)) {
        spawn(
          regPath,
          ['add', regKey, '/v', 'IconUri', '/t', 'REG_SZ', '/d', iconPath, '/f'],
          (iconErr) => {
            if (iconErr) {
              console.warn('Failed to register AUMID IconUri:', iconErr);
            }
            if (callback) callback(err || iconErr);
          }
        );
      } else {
        if (callback) callback(err);
      }
    }
  );
}

exports.registerAppUserModelId = registerAppUserModelId;
exports.APP_USER_MODEL_ID = APP_USER_MODEL_ID;
exports.TOAST_ACTIVATOR_CLSID = TOAST_ACTIVATOR_CLSID;

// Copy Start Menu tile visual elements (icon PNGs + manifest XML) from the
// current app-x.x.x/resources directory to the root install directory so
// Windows can display a branded tile. Errors are ignored — these are optional.
function copyVisualElements() {
  try {
    const files = ['tern-75px.png', 'tern-150px.png', 'tern.VisualElementsManifest.xml'];
    for (const file of files) {
      fs.copyFileSync(path.join(appFolder, 'resources', file), path.join(rootAppDataFolder, file));
    }
  } catch (err) {
    // Ignore errors - visual elements are optional
  }
}

// Restart Tern using the version pointed to by the Squirrel Update.exe shim.
// Uses spawnDetached to ensure the child process survives the parent's exit —
// the piped-stdio `spawn` function can fail when called during `will-quit`
// because the Node.js event loop tears down the pipe before Update.exe launches
// the new app instance.
//
// Uses --processStartAndWait (not --processStart) so that Update.exe waits for
// the current instance to fully release requestSingleInstanceLock() before
// launching the new one. Without the Wait variant, the new instance can start
// before the old one exits, hit the single-instance lock, and immediately quit
// — leaving no running instance. See: https://github.com/electron/electron/pull/6037
exports.restartMailspring = (app) => {
  app.once('will-quit', () => {
    spawnDetached(updateDotExe, ['--processStartAndWait', exeName]);
  });
  app.quit();
};

// Squirrel's own --createShortcut stamps shortcuts with "com.squirrel.<id>.<exe>" as their
// AppUserModelID and no toast activator, which wouldn't match APP_USER_MODEL_ID (set in
// main.js), and Windows only delivers toasts and groups taskbar buttons for an AUMID that a
// Start Menu shortcut carries. So Tern writes its shortcuts itself.
function shortcutPaths() {
  return [
    path.join(process.env.APPDATA, 'Microsoft', 'Windows', 'Start Menu', 'Programs', 'Tern.lnk'),
    path.join(process.env.USERPROFILE || process.env.HOME, 'Desktop', 'Tern.lnk'),
  ];
}

function writeShortcuts({ replaceExisting }) {
  const iconPath = path.join(appFolder, 'resources', 'tern.ico');
  const shortcutOptions = {
    // Update.exe --processStart always launches the newest installed app-x.x.x.
    target: updateDotExe,
    args: `--processStart ${exeName}`,
    icon: fs.existsSync(iconPath) ? iconPath : undefined,
    iconIndex: 0,
    description: 'The best email app for people and teams at work',
    appUserModelId: APP_USER_MODEL_ID,
    toastActivatorClsid: TOAST_ACTIVATOR_CLSID,
  };
  for (const shortcutPath of shortcutPaths()) {
    try {
      if (replaceExisting || !fs.existsSync(shortcutPath)) {
        shell.writeShortcutLink(shortcutPath, 'create', shortcutOptions);
      }
    } catch (err) {
      // Ignore - a missing shortcut is not worth failing the install for
    }
  }
}

// Handle --squirrel-install event with fast exit.
// Squirrel.Windows has a 15-second timeout for hooks. We spawn all necessary
// processes in detached mode and exit immediately to avoid timeout.
// See: https://github.com/Squirrel/Squirrel.Windows/issues/501
// See: https://github.com/Squirrel/Squirrel.Windows/issues/1145
exports.handleSquirrelInstall = (app) => {
  copyVisualElements();
  writeShortcuts({ replaceExisting: true });

  // Spawn reg.exe to register AUMID (detached - won't block exit)
  const iconPath = path.join(appFolder, 'resources', 'tern.ico');
  const regKey = `HKEY_CURRENT_USER\\SOFTWARE\\Classes\\AppUserModelId\\${APP_USER_MODEL_ID}`;
  let regPath = 'reg.exe';
  if (process.env.SystemRoot) {
    regPath = path.join(process.env.SystemRoot, 'System32', 'reg.exe');
  }
  spawnDetached(regPath, ['add', regKey, '/v', 'DisplayName', '/t', 'REG_SZ', '/d', 'Tern', '/f']);
  if (fs.existsSync(iconPath)) {
    spawnDetached(regPath, ['add', regKey, '/v', 'IconUri', '/t', 'REG_SZ', '/d', iconPath, '/f']);
  }

  // Registry entries for mailto: protocol are registered on first normal app launch
  // (via createRegistryEntries call in main.js startup). This ensures registration
  // completes even if the detached processes here don't finish before Squirrel's timeout.

  // Exit immediately - don't wait for spawned processes
  app.quit();
};

// Handle --squirrel-updated event with fast exit.
// Squirrel runs the NEW app version with this flag after extracting an update.
// The shortcuts point at Update.exe, so they only need recreating if the user deleted them,
// which the update leaves alone. The actual app restart happens later when the user clicks
// "Install Update".
exports.handleSquirrelUpdated = (app) => {
  copyVisualElements();
  writeShortcuts({ replaceExisting: false });

  // Exit immediately - don't wait for spawned processes
  app.quit();
};

// Handle --squirrel-uninstall event with fast exit.
exports.handleSquirrelUninstall = (app) => {
  for (const shortcutPath of shortcutPaths()) {
    try {
      if (fs.existsSync(shortcutPath)) {
        fs.unlinkSync(shortcutPath);
      }
    } catch (err) {
      // Ignore
    }
  }

  // Exit immediately
  app.quit();
};
