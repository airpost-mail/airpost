export {};
declare global {
  // Set on the main process's global by main.js.
  // eslint-disable-next-line no-var
  var shellStartTime: number;
  // eslint-disable-next-line no-var
  var application: import('./application').default;
  // The main process's ErrorLogger (platform/error-logger.js); windows use AppEnv.errorLogger.
  // eslint-disable-next-line no-var
  var errorLogger: { reportError(error: unknown, extra?: object): void };
}
