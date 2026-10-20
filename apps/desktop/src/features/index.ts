// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/* eslint global-require: 0 */

// The built-in features: app code that always loads, one directory each. PackageManager merges
// them by `dir` with the bundled plugins and themes and activates the lot in that name order,
// so toolbar buttons, message headers and composer extensions register in the same order as
// when the features were packages.
//
// `dir` is also the `mailspring://<dir>/…` asset host, so it must not change. `name` is the
// name the feature has in AppEnv.packages (defaults to `dir`). `load` stays a lazy require:
// many feature modules have import-time side effects that only belong in the windows that
// activate them.

export interface FeatureWindowTypes {
  [windowType: string]: boolean;
}

export interface FeatureManifest {
  dir: string;
  name?: string;
  // The window types it activates in; `all` matches every one.
  windowTypes: FeatureWindowTypes;
  // Activate at once instead of in the deferred batch 2.5 s later.
  syncInit?: boolean;
  load: () => {
    activate: () => void;
    deactivate: () => void;
    config?: object;
    configDefaults?: object;
    activateConfig?: () => void;
  };
}

export const FEATURES: FeatureManifest[] = [
  {
    dir: 'account-sidebar',
    windowTypes: { default: true },
    syncInit: true,
    load: () => require('./account-sidebar'),
  },
  {
    dir: 'attachments',
    windowTypes: { default: true, composer: true, 'thread-popout': true },
    syncInit: true,
    load: () => require('./attachments'),
  },
  {
    dir: 'category-mapper',
    windowTypes: { default: true },
    load: () => require('./category-mapper'),
  },
  {
    dir: 'category-picker',
    windowTypes: { default: true },
    load: () => require('./category-picker'),
  },
  {
    dir: 'composer',
    windowTypes: { default: true, composer: true, 'thread-popout': true },
    syncInit: true,
    load: () => require('./composer'),
  },
  {
    dir: 'composer-signature',
    windowTypes: { default: true, composer: true, 'thread-popout': true },
    load: () => require('./composer-signature'),
  },
  {
    dir: 'contacts',
    windowTypes: { contacts: true },
    syncInit: true,
    load: () => require('./contacts'),
  },
  {
    dir: 'custom-fonts',
    windowTypes: { all: true },
    syncInit: true,
    load: () => require('./custom-fonts'),
  },
  {
    dir: 'custom-sounds',
    windowTypes: { all: true },
    load: () => require('./custom-sounds'),
  },
  {
    dir: 'draft-list',
    windowTypes: { default: true },
    load: () => require('./draft-list'),
  },
  {
    dir: 'events',
    windowTypes: { default: true, 'thread-popout': true },
    load: () => require('./events'),
  },
  {
    dir: 'list-unsubscribe',
    name: 'unsubscribe',
    windowTypes: { default: true, 'thread-popout': true },
    load: () => require('./list-unsubscribe'),
  },
  {
    dir: 'main-calendar',
    windowTypes: { default: true },
    syncInit: true,
    load: () => require('./main-calendar'),
  },
  {
    dir: 'message-autoload-images',
    windowTypes: { default: true, 'thread-popout': true },
    syncInit: true,
    load: () => require('./message-autoload-images'),
  },
  {
    dir: 'message-list',
    windowTypes: { default: true, 'thread-popout': true },
    syncInit: true,
    load: () => require('./message-list'),
  },
  {
    dir: 'mode-switch',
    windowTypes: { default: true },
    load: () => require('./mode-switch'),
  },
  {
    dir: 'notifications',
    windowTypes: { default: true },
    syncInit: true,
    load: () => require('./notifications'),
  },
  {
    dir: 'onboarding',
    windowTypes: { onboarding: true },
    load: () => require('./onboarding'),
  },
  {
    dir: 'preferences',
    windowTypes: { default: true },
    syncInit: true,
    load: () => require('./preferences'),
  },
  {
    dir: 'print',
    windowTypes: { default: true, 'thread-popout': true },
    load: () => require('./print'),
  },
  {
    dir: 'remove-tracking-pixels',
    windowTypes: { default: true, 'thread-popout': true },
    load: () => require('./remove-tracking-pixels'),
  },
  {
    dir: 'send-and-archive',
    windowTypes: { default: true, composer: true, 'thread-popout': true, work: true },
    load: () => require('./send-and-archive'),
  },
  {
    dir: 'system-tray',
    windowTypes: { default: true },
    load: () => require('./system-tray'),
  },
  {
    dir: 'theme-picker',
    windowTypes: { default: true },
    load: () => require('./theme-picker'),
  },
  {
    dir: 'thread-list',
    windowTypes: { default: true },
    syncInit: true,
    load: () => require('./thread-list'),
  },
  {
    dir: 'thread-search',
    windowTypes: { default: true },
    syncInit: true,
    load: () => require('./thread-search'),
  },
  {
    dir: 'undo-redo',
    windowTypes: { default: true, 'thread-popout': true },
    load: () => require('./undo-redo'),
  },
  {
    dir: 'unread-notifications',
    windowTypes: { default: true },
    load: () => require('./unread-notifications'),
  },
];
