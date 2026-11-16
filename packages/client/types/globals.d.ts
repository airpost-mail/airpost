// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

// The globals the browser-safe part of the package uses, which every runtime it targets has
// (browsers, Node, React Native). Only tsconfig.json includes this, so a use of anything
// else (DOM or Node) fails the package's typecheck.
type TimerHandle = { readonly __timer: unique symbol } | number;
declare function setTimeout(handler: () => void, ms?: number): TimerHandle;
declare function clearTimeout(handle: TimerHandle | undefined): void;
declare const console: {
  log(...args: unknown[]): void;
  warn(...args: unknown[]): void;
  error(...args: unknown[]): void;
};
