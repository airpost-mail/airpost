export {};

declare global {
  const AppEnv: import('./app-env').default;

  // AppEnv.enhanceEventObject adds these to every Event in a window.
  interface Event {
    propagationStopped?: boolean;
    isPropagationStopped(): boolean | undefined;
  }

  interface Window {
    $m: any;
    AppEnv: import('./app-env').default;
    jasmine: any;
    // requestIdleCallback and cancelIdleCallback are now built-in to TypeScript's DOM lib
  }
}
