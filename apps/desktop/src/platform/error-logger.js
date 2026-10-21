var ErrorLogger, _;

let ipcRenderer = null;
if (process.type === 'renderer') {
  ipcRenderer = require('electron').ipcRenderer;
}

// A globally available ErrorLogger that can report errors to various
// sources and enhance error functionality.
//
// This runs in both the backend browser process and each and every
// renderer process.
//
// This is available as `global.errorLogger` in the backend browser
// process.
//
// It is available at `AppEnv.errorLogger` in each renderer process.
// You should almost always use `AppEnv.reportError` in the renderer
// processes instead of manually accessing the `errorLogger`
//
// The errorLogger logs errors to the console; the main process also keeps
// native crash dumps on disk.
module.exports = ErrorLogger = (function () {
  function ErrorLogger(args) {
    this.reportError = this.reportError.bind(this);
    this.inSpecMode = args.inSpecMode;
    this.inDevMode = args.inDevMode;
    this.resourcePath = args.resourcePath;

    this._startCrashReporter();

    this._extendErrorObject();

    this._extendNativeConsole();
  }

  /////////////////////////////////////////////////////////////////////
  /////////////////////////// PUBLIC METHODS //////////////////////////
  /////////////////////////////////////////////////////////////////////

  ErrorLogger.prototype.reportError = function (error, extra = {}) {
    if (this.inSpecMode) {
      return;
    }
    // An error without a stack gets one from wherever `new Error()` runs —
    // for JSON reparsed in the main process's `report-error` IPC handler (see
    // application.ts), that's the same IPC handler frame for every caller,
    // whatever the error's real origin. Wrap such inputs in a real Error
    // *here*, in the renderer, before the IPC hop, so the stack points at
    // the caller. Keep the original message when there is one — only the
    // stack is synthetic.
    var hasMessage =
      error && typeof error.message === 'string' && error.message.length > 0;
    var hasStack = error && typeof error.stack === 'string' && error.stack.length > 0;
    if (!hasStack) {
      var message;
      if (hasMessage) {
        message = error.message;
      } else {
        var description;
        try {
          description = JSON.stringify(error);
        } catch (e) {
          description = Object.prototype.toString.call(error);
        }
        message = 'Empty error reported (' + description + ')';
      }
      var wrapped = new Error(message);
      if (error && typeof error === 'object') {
        // Copy any other useful fields from the original, but never let a
        // missing `stack` (or, when we just fell back to the description
        // above, an empty `message`) on the input clobber the wrap's real
        // values — that would defeat the whole purpose of wrapping.
        try {
          var keys = Object.getOwnPropertyNames(error);
          for (var i = 0; i < keys.length; i++) {
            var key = keys[i];
            if (key === 'message' || key === 'stack') continue;
            wrapped[key] = error[key];
          }
        } catch (e) {
          // ignore non-assignable inputs
        }
      }
      error = wrapped;
    }
    if (process.type === 'renderer') {
      var errorJSON = '{}';
      try {
        errorJSON = JSON.stringify(error);
      } catch (err) {
        var recoveredError = new Error();
        recoveredError.stack = error.stack;
        recoveredError.message = `Recovered Error: ${error.message}`;
        errorJSON = JSON.stringify(recoveredError);
      }

      var extraJSON;
      try {
        extraJSON = JSON.stringify(extra);
      } catch (err) {
        extraJSON = '{}';
      }

      /**
       * We synchronously send all errors to the backend main process.
       *
       * This is important because errors can frequently happen right
       * before a renderer window is closing. Since error reporting hits
       * APIs and is asynchronous it's possible for the window to be
       * destroyed before the report makes it.
       *
       * This is a rare use of `sendSync` to ensure the command has made
       * it before the window closes.
       */
      ipcRenderer.sendSync('report-error', { errorJSON: errorJSON, extra: extraJSON });
    }
    // console.error can itself throw (eg. EPIPE writing to a closed
    // stdout/stderr pipe on Linux). Since reportError is called from our
    // top-level uncaughtException handler, an uncaught throw here would
    // re-enter that same handler and loop forever. Never let this fail.
    try {
      console.error(error, extra);
    } catch (loggingError) {
      // ignore
    }
  };

  /////////////////////////////////////////////////////////////////////
  ////////////////////////// PRIVATE METHODS //////////////////////////
  /////////////////////////////////////////////////////////////////////

  ErrorLogger.prototype._startCrashReporter = function (args) {
    if (process.type === 'renderer') {
      return;
    }
    // Native crash dumps stay on disk (app.getPath('crashDumps')); there is no endpoint to
    // upload them to.
    require('electron').crashReporter.start({
      productName: 'Tern',
      companyName: 'Tern',
      uploadToServer: false,
      extra: {
        ver: require('electron').app.getVersion(),
        platform: process.platform,
      },
    });
  };

  ErrorLogger.prototype._extendNativeConsole = function (args) {
    console.debug = this._consoleDebug.bind(this);
  };

  // globally define Error.toJSON. This allows us to pass errors via IPC
  // and through the Action Bridge. Note:they are not re-inflated into
  // Error objects automatically.
  ErrorLogger.prototype._extendErrorObject = function (args) {
    Object.defineProperty(Error.prototype, 'toJSON', {
      value: function () {
        var alt = {};

        Object.getOwnPropertyNames(this).forEach(function (key) {
          alt[key] = this[key];
        }, this);

        return alt;
      },
      configurable: true,
    });
  };

  // Create a new console.debug option, which takes `true` (print)
  // or `false`, don't print in console as the first parameter.
  // This makes it easy for developers to turn on and off
  // "verbose console" mode.
  ErrorLogger.prototype._consoleDebug = function () {
    var args = [];
    var showIt = arguments[0];
    for (var ii = 1; ii < arguments.length; ii++) {
      args.push(arguments[ii]);
    }
    if (this.inDevMode === true && showIt === true) {
      console.log.apply(console, args);
    }
  };

  return ErrorLogger;
})();
