/* eslint dot-notation: 0 */
/* eslint global-require: 0 */
global.shellStartTime = Date.now();
const util = require('util');

const fs = require('fs');

// On Linux, writes to process.stdout/stderr use a synchronous fast path when
// the fd is a pipe or a plain file. If the destination goes away or becomes
// unwritable — the reader end of a pipe exits (EPIPE), or the fd is backed by
// a filesystem that's read-only or gone (EROFS, seen eg. under snap when a
// revision's squashfs mount is swapped out from under a running process, or
// EIO/ENOSPC/EBADF for similar reasons) — the write throws synchronously
// instead of emitting an event, because there's no 'error' listener on the
// stream. When this happens inside our `uncaughtException` handler (which
// itself logs via console.error), the thrown error re-enters the same
// handler, producing a crash loop. None of these codes are actionable by the
// app, so swallow them all here and let writes to a dead stdout/stderr
// silently no-op.
const IGNORABLE_STREAM_ERROR_CODES = new Set(['EPIPE', 'EROFS', 'EIO', 'ENOSPC', 'EBADF']);
for (const stream of [process.stdout, process.stderr]) {
  if (stream) {
    stream.on('error', error => {
      if (error && IGNORABLE_STREAM_ERROR_CODES.has(error.code)) {
        return;
      }
      throw error;
    });
  }
}

console.inspect = function consoleInspect(val) {
  console.log(util.inspect(val, true, 7, true));
};

const { app, session, protocol } = require('electron');
const path = require('path');
const { developerToolsAvailable } = require('../platform/developer-tools');
const { isSamePath } = require('./same-path');

if (typeof process.setFdLimit === 'function') {
  process.setFdLimit(1024);
}

// Tern keeps its own profile so it can run next to Mailspring.app. The app name (package.json
// productName) is "Tern", so safeStorage's keychain item is "Tern Safe Storage" ("Tern Dev Safe
// Storage" unpackaged, see start()).
const profileDirName = args => {
  if (args.specMode) return 'Tern-spec';
  if (args.devMode) return 'Tern-dev';
  return 'Tern';
};

// Returns the config dir, the Mailspring profile Import from Mailspring offers (null when there
// is none to offer), and whether the dir is Tern's own (profile/marker.ts): the default one, or
// a --config-dir-path that didn't exist yet or was empty.
const setupConfigDir = args => {
  const appData = app.getPath('appData');
  const snap = process.platform === 'linux' && !!process.env.SNAP;
  let configDirPath = args.configDirPath || path.join(appData, profileDirName(args));
  // A snap can't see Mailspring's profile outside its sandbox, and specs never offer the real one.
  const mailspringProfilePath = args.specMode || snap ? null : path.join(appData, 'Mailspring');

  if (snap) {
    // for linux snap, use the sandbox directory that is persisted between snap revisions
    configDirPath = args.configDirPath || process.env.SNAP_USER_COMMON;
  }

  const ownsConfigDir =
    !args.configDirPath ||
    !fs.existsSync(configDirPath) ||
    (fs.statSync(configDirPath).isDirectory() && fs.readdirSync(configDirPath).length === 0);
  fs.mkdirSync(configDirPath, { recursive: true });

  // Also moves Electron's local storage and crash dumps here, for --dev and --config-dir-path
  // too (the default userData would be the app name's folder, "Tern").
  app.setPath('userData', configDirPath);

  return { configDirPath, mailspringProfilePath, ownsConfigDir };
};

// Starts the Import from Mailspring the last session asked for, or that
// --import-from-mailspring names for a new profile (profile/mailspring-import.ts). Resolves once
// the profile is ready to use: to null when there was nothing to import, else to the result or
// the error (with nothing copied left behind).
const startProfileImport = (configDirPath, importFromMailspring) => {
  const imports = require('./profile/mailspring-import');
  const from = imports.pendingMailspringImport(configDirPath, importFromMailspring);
  if (!from) return Promise.resolve(null);
  const credentials = stored =>
    app.whenReady().then(() => importFlow(configDirPath).mailspringCredentialsForTern(stored, from));
  return imports
    .importMailspringProfile(from, configDirPath, { credentials })
    .then(result => ({ result }), error => ({ error }));
};

// The import's dialogs, once `ready`, in the profile's language (Application.start loads it
// again once config.json is in place).
const importFlow = configDirPath => {
  require('../intl/localize').initializeLocalization({ configDirPath });
  return require('./profile/mailspring-import-flow');
};

const setupCompileCache = (configDirPath, devMode) => {
  if (devMode) {
    require('../platform/compile-cache-ts').setHomeDirectory(configDirPath);
  } else {
    require('../platform/compile-cache-ts-unsupported');
  }
};

const setupErrorLogger = (args = {}) => {
  const ErrorLogger = require('../platform/error-logger');
  const errorLogger = new ErrorLogger({
    inSpecMode: args.specMode,
    inDevMode: args.devMode,
    resourcePath: args.resourcePath,
  });
  process.on('uncaughtException', (error, origin) => errorLogger.reportError(error, { origin }));
  process.on('unhandledRejection', reason => errorLogger.reportError(reason));
  return errorLogger;
};

const declareOptions = argv => {
  const optimist = require('optimist');
  const options = optimist(argv);
  options.usage(
    `Tern\n\nUsage: tern [options] [recipient] [attachment]\n\nRun Tern: The open source extensible email client\n\n\`tern mailto:johndoe@example.com\` to compose an e-mail to johndoe@example.com.\n\`tern ./attachment.txt\` to compose an e-mail with a text file attached.\n\`tern --dev\` to start the client in dev mode.\n\`tern --test\` to run unit tests.`
  );
  options
    .alias('d', 'dev')
    .boolean('d')
    .describe('d', 'Run in development mode.');
  options
    .alias('t', 'test')
    .boolean('t')
    .describe('t', 'Run the specified specs and exit with error code on failures.');
  options
    .boolean('safe')
    .describe(
      'safe',
      'Do not load packages from the settings `packages` or `dev/packages` folders.'
    );
  // The options --enable-crashpad and --allow-file-access-from-files are added to the command line options by electron when opening a second instance of Tern.
  // If they are not defined as boolean options here, they will "swallow" every argument that is passed after them. This leads to the "Send To" functionality not working
  // if Tern is already running.
  options.boolean('enable-crashpad');
  options.boolean('allow-file-access-from-files');
  options.boolean('source-app-id');
  options
    .alias('h', 'help')
    .boolean('h')
    .describe('h', 'Print this usage message.');
  options
    .alias('l', 'log-file')
    .string('l')
    .describe('l', 'Log all test output to file.');
  options
    .alias('c', 'config-dir-path')
    .string('c')
    .describe('c', 'Override the path to the Tern configuration directory');
  options
    .alias('s', 'spec-directory')
    .string('s')
    .describe(
      's',
      "Run the tests in this directory (*.test.* and *-spec.* files) instead of the app's"
    );
  options
    .alias('f', ['spec-file-pattern', 'test-file-pattern'])
    .string('f')
    .describe(
      'f',
      'Only run the test files whose path (relative to the app, or to --spec-directory) matches this regex'
    );
  options
    .alias('v', 'version')
    .boolean('v')
    .describe('v', 'Print the version.');
  options
    .alias('b', 'background')
    .boolean('b')
    .describe('b', 'Start Tern in the background');
  options
    .string('import-from-mailspring')
    .describe(
      'import-from-mailspring',
      'Import this Mailspring profile into a Tern profile that has no config.json yet'
    );
  return options;
};

const parseCommandLine = argv => {
  const pkg = require('../../package.json');
  const version = `${pkg.version}-${pkg.commitHash}`;

  const options = declareOptions(argv.slice(1));
  const args = options.argv;

  if (args.help) {
    process.stdout.write(options.help());
    process.exit(0);
  }
  if (args.version) {
    process.stdout.write(`${version}\n`);
    process.exit(0);
  }
  // A packaged build silently ignores both flags rather than letting a release be switched
  // into development mode from the command line. See developerToolsAvailable().
  const developerBuild = developerToolsAvailable();
  const devMode = developerBuild && (args['dev'] || args['test']);
  const logFile = args['log-file'];
  const specMode = developerBuild && args['test'];
  const jUnitXmlPath = args['junit-xml'];
  const safeMode = args['safe'];
  const background = args['background'];
  const configDirPath = args['config-dir-path'];
  const importFromMailspring = args['import-from-mailspring'] || null;
  const specDirectory = args['spec-directory'] && path.resolve(args['spec-directory']);
  if (specDirectory && !fs.existsSync(specDirectory)) {
    process.stderr.write(`--spec-directory does not exist: ${specDirectory}\n`);
    process.exit(1);
  }
  const specFilePattern = args['spec-file-pattern'];
  const showSpecsInWindow = specMode === 'window';
  const resourcePath = path.normalize(path.resolve(path.dirname(path.dirname(__dirname))));
  let urlsToOpen = [];
  let pathsToOpen = [];

  // On Windows and Linux, mailto and file opens are passed in argv. Go through
  // the items and pluck out things that look like mailto:, tern:, file paths
  let ignoreNext = false;
  // args._ is all of the non-hyphenated options.
  for (const arg of args._) {
    if (ignoreNext) {
      ignoreNext = false;
      continue;
    }
    if (arg.includes('executed-from') || arg.includes('squirrel')) {
      ignoreNext = true;
      continue;
    }
    // Skip the argument if it's part of the main electron invocation. __dirname has its
    // symlinks resolved and the argument may not (/tmp on macOS), so compare real paths.
    if (isSamePath(arg, resourcePath)) {
      continue;
    }
    if (arg.startsWith('mailto:') || arg.startsWith('tern:')) {
      urlsToOpen.push(arg);
    } else if (arg[0] !== '-' && arg[0] !== '?' && /[/|\\]/.test(arg)) {
      pathsToOpen.push(arg);
    }
  }

  if (args['path-environment']) {
    process.env.PATH = args['path-environment'];
  }

  return {
    version,
    devMode,
    background,
    logFile,
    specMode,
    jUnitXmlPath,
    safeMode,
    configDirPath,
    importFromMailspring,
    specDirectory,
    specFilePattern,
    showSpecsInWindow,
    resourcePath,
    urlsToOpen,
    pathsToOpen,
  };
};

/*
 * "Squirrel will spawn your app with command line flags on first run, updates,
 * and uninstalls."
 *
 * Read: https://github.com/electron-archive/grunt-electron-installer#handling-squirrel-events
 * Read: https://github.com/electron/electron/blob/master/docs/api/auto-updater.md#windows
 *
 * IMPORTANT: Squirrel.Windows has a 15-second timeout for hooks (10 seconds for uninstall).
 * If the app doesn't exit within that time, Squirrel cancels with OperationCanceledException.
 * We must handle events quickly and exit immediately - spawning any long-running processes
 * in detached mode so they continue after the main process exits.
 *
 * See: https://github.com/Squirrel/Squirrel.Windows/issues/501
 * See: https://github.com/Squirrel/Squirrel.Windows/issues/1145
 */
const handleStartupEventWithSquirrel = () => {
  if (process.platform !== 'win32') {
    return false;
  }

  const WindowsUpdater = require('./autoupdate/win32-updater');
  const squirrelCommand = process.argv[1];

  switch (squirrelCommand) {
    case '--squirrel-install':
      // Handle install with fast exit - spawns detached processes and quits immediately
      WindowsUpdater.handleSquirrelInstall(app);
      return true;
    case '--squirrel-updated':
      // Squirrel runs the NEW version with this flag after applying an update.
      // Per Squirrel.Windows conventions, we should update shortcuts and exit
      // quickly — NOT restart the app. The restart happens later when the user
      // triggers "Install Update" via the UI, which calls restartApp().
      // Calling restartApp() here would spawn a new instance that
      // requestSingleInstanceLock() kills (the original app is still
      // running), wasting time and risking Squirrel's 15s timeout.
      WindowsUpdater.handleSquirrelUpdated(app);
      return true;
    case '--squirrel-uninstall':
      // Handle uninstall with fast exit - spawns detached processes and quits immediately
      WindowsUpdater.handleSquirrelUninstall(app);
      return true;
    case '--squirrel-obsolete':
      app.quit();
      return true;
    default:
      return false;
  }
};

const start = () => {
  if (process.platform === 'win32') {
    // Must be set before setAppUserModelId so RegisterActivator writes it
    // into the Start Menu shortcut. Without this, action/reply notification
    // events are silently dropped (COM server is never registered).
    const WindowsUpdater = require('./autoupdate/win32-updater');
    app.setToastActivatorCLSID(WindowsUpdater.TOAST_ACTIVATOR_CLSID);
    app.setAppUserModelId(WindowsUpdater.APP_USER_MODEL_ID);
  }

  // Set the app name explicitly for Linux to ensure the system tray icon
  // gets a unique ID. Without this, all Electron apps share the same
  // StatusNotifierItem ID on Linux, causing their tray visibility settings
  // to be synchronized. See: https://github.com/electron/electron/issues/40936
  if (process.platform === 'linux') {
    app.setName('Tern');
  }
  // safeStorage's key is "<app name> Safe Storage" (the Keychain item; "<app name> Keys" on
  // Linux), read only by the build that made it. An unpackaged run sharing the release's name
  // would leave the release a key it can't read. The profile dirs don't follow the name.
  if (!app.isPackaged) {
    app.setName('Tern Dev');
  }


  protocol.registerSchemesAsPrivileged([
    // Package assets (main/protocols/asset.ts); mailspring: is the v1 plugin API name.
    ...['tern-asset', 'mailspring'].map((scheme) => ({
      scheme,
      privileges: {
        secure: true,
        supportFetchAPI: true,
        corsEnabled: true,
      },
    })),
    {
      // Downloaded attachments shown in message bodies (src/mail/files-scheme.ts).
      scheme: 'tern-file',
      privileges: {
        secure: true,
      }
    },
    {
      // The sandboxed frame message bodies render in (src/mail/frame). Standard, so the
      // frame has an origin of its own and runs in its own sandboxed renderer process.
      scheme: 'tern-message',
      privileges: {
        standard: true,
        secure: true,
      }
    }
  ])

  if (handleStartupEventWithSquirrel()) {
    return;
  }

  // On Windows, register the AppUserModelId with a display name so notifications
  // show "Tern" instead of the raw AUMID.
  // Also register mailto: protocol handler so Windows knows Tern can handle
  // mailto: links (this doesn't make it the default, just registers it as an option).
  // This handles existing installations and ensures registration completes even if
  // the Squirrel install hook's detached processes didn't finish in time.
  if (process.platform === 'win32') {
    const WindowsUpdater = require('./autoupdate/win32-updater');
    if (WindowsUpdater.existsSync()) {
      WindowsUpdater.registerAppUserModelId();
      // Register mailto: protocol without elevation or setting as default
      WindowsUpdater.createRegistryEntries(
        { allowEscalation: false, registerDefaultIfPossible: false },
        () => {}
      );
    }
  }

  // Tern's code doesn't use @electron/remote (src/main/ipc/app-api.ts); it stays for v1
  // plugins that load it, until plugin API 2.0.
  require('@electron/remote/main').initialize();

  // Configure Chromium command line switches before app ready event.
  // These must be set before the ready event for them to take effect.
  // Reference: https://www.electronjs.org/docs/latest/api/command-line-switches
  // New-mail sounds play with no user gesture (src/packages/sound-registry.ts).
  app.commandLine.appendSwitch('autoplay-policy', 'no-user-gesture-required');

  const options = parseCommandLine(process.argv);
  // Before the error logger: it starts the crash reporter, which writes under userData.
  const { configDirPath, mailspringProfilePath, ownsConfigDir } = setupConfigDir(options);
  global.errorLogger = setupErrorLogger(options);
  options.configDirPath = configDirPath;
  options.mailspringProfilePath = mailspringProfilePath;

  // On macOS, setLoginItemSettings doesn't support passing custom args, so we
  // detect login-item launches via wasOpenedAtLogin and start in background.
  if (process.platform === 'darwin' && !options.background) {
    const settings = app.getLoginItemSettings();
    if (settings.wasOpenedAtLogin) {
      options.background = true;
    }
  }

  if (!options.devMode) {
    const gotTheLock = app.requestSingleInstanceLock();

    if (!gotTheLock) {
      console.log('Exiting because another instance of the app is already running.');
      app.exit(1);
      return;
    }

    app.on('second-instance', (event, commandLine, workingDirectory) => {
      const otherOpts = parseCommandLine(commandLine);
      global.application.handleLaunchOptions(otherOpts);
    });
  }

  setupCompileCache(configDirPath, options.devMode);
  if (ownsConfigDir) require('./profile/marker').markTernProfile(configDirPath);

  // Every session gets the deny-by-default permission handlers as it is created: the default
  // one, quickpreview's and the <webview> partition, and any a plugin opens.
  const installPermissionHandlers = ses => {
    require('./permission-policy').installPermissionHandlers(ses, {
      isAppWindow: require('./windows/app-window').isAppWindowContents,
      isPdfViewer: require('./ipc/quick-preview').isPdfViewerURL,
      logDenials: options.devMode,
    });
  };
  app.on('session-created', installPermissionHandlers);

  const profileImport = startProfileImport(configDirPath, options.importFromMailspring);

  // Must precede `ready`: disableHardwareAcceleration() has no effect afterward.
  require('./windows/hardware-acceleration-recovery').applyPersistentSoftwareRendering(
    app,
    configDirPath
  );

  const onOpenFileBeforeReady = (event, file) => {
    event.preventDefault();
    options.pathsToOpen.push(file);
  };

  const onOpenUrlBeforeReady = (event, url) => {
    event.preventDefault();
    options.urlsToOpen.push(url);
  };

  app.on('open-url', onOpenUrlBeforeReady);
  app.on('open-file', onOpenFileBeforeReady);
  app.on('ready', async () => {
    const imported = await profileImport;
    if (imported) importFlow(configDirPath).reportProfileImport(imported);

    app.removeListener('open-file', onOpenFileBeforeReady);
    app.removeListener('open-url', onOpenUrlBeforeReady);

    // In case the default session was created before the listener above.
    installPermissionHandlers(session.defaultSession);

    // No window may reach file://host/... (a UNC path on Windows), which the app CSP's `*`,
    // `file:` and 'self' all allow from a file: document. Message HTML also renders in the
    // main window (the quoted body in the composer), where no per-message CSP applies.
    const { isHostedFileURL } = require('../utils/file-url');
    session.defaultSession.webRequest.onBeforeRequest((details, callback) => {
      callback({ cancel: isHostedFileURL(details.url) });
    });

    // Remove the Origin header for Microsoft OAuth requests. Native fetch in Electron
    // adds an Origin header which causes AADSTS90023 errors because Microsoft treats
    // it as a cross-origin request requiring SPA client-type registration. Desktop apps
    // should not send Origin headers for OAuth token exchange.
    const o365Filter = {
      urls: ['*://login.microsoftonline.com/*'],
    };

    session.defaultSession.extensions
      .loadExtension(
        path
          .join(options.resourcePath, 'static', 'extensions', 'chrome-i18n')
          .replace('app.asar', 'app.asar.unpacked'),
        { allowFileAccess: true }
      )
      .catch(err => console.error(`Error loading language detection extension: ${err}`));

    session.defaultSession.webRequest.onBeforeSendHeaders(o365Filter, (details, callback) => {
      delete details.requestHeaders['Origin'];
      callback({ requestHeaders: details.requestHeaders });
    });

    // The CSP also travels as a header in case the <meta> tag in static/index.html is missing
    // or removed. Both come from AppContentSecurityPolicy.
    const { AppContentSecurityPolicy } = require('../mail/content-security-policy');
    session.defaultSession.webRequest.onHeadersReceived((details, callback) => {
      // The message frame sends its own, stricter policy.
      if (details.url.startsWith('devtools://') || details.url.startsWith('tern-message:')) {
        return callback({});
      }
      callback({
        responseHeaders: {
          ...details.responseHeaders,
          'Content-Security-Policy': [AppContentSecurityPolicy],
        },
      });
    });

    // eslint-disable-next-line
    const Application = require(path.join(options.resourcePath, 'src', 'main', 'application'))
      .default;
    global.application = new Application();
    global.application.start(options);

    if (!options.specMode) {
      console.log(`App load time: ${Date.now() - global.shellStartTime}ms`);
    }
  });
};

start();
