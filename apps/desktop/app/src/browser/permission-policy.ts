// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Web permissions, denied unless asked for by the main frame of one of Tern's own windows, and then
only the ones the app uses. Without a handler Electron grants nearly everything (camera,
microphone, geolocation, clipboard-read, screen capture) to any frame, message iframes included.

- clipboard-sanitized-write: navigator.clipboard.writeText (CopyButton, thread sharing).
- notifications: the web Notification API, for plugins. Tern's own notifications are shown from
  main (notification-ipc.ts) and need no permission.

Sounds (SoundRegistry) play through an Audio element, which needs no permission; they do need
the autoplay-policy switch in main.js, since a new-mail sound plays with no user gesture.
*/

export const AppWindowPermissions: ReadonlySet<string> = new Set([
  'clipboard-sanitized-write',
  'notifications',
]);

export interface PermissionAsk {
  permission: string;
  // The asking webContents is one of Tern's windows (isMailspringWindowContents).
  appWindow: boolean;
  isMainFrame: boolean;
  requestingUrl?: string;
}

export function permissionAllowed({
  permission,
  appWindow,
  isMainFrame,
  requestingUrl,
}: PermissionAsk) {
  return (
    appWindow &&
    isMainFrame &&
    /^file:/i.test(requestingUrl || '') &&
    AppWindowPermissions.has(permission)
  );
}

type PermissionSession = Pick<
  Electron.Session,
  'setPermissionRequestHandler' | 'setPermissionCheckHandler'
>;

export interface PermissionHandlerOptions {
  isAppWindow: (contents: Electron.WebContents) => boolean;
  // Dev mode: each denied permission and URL is logged once.
  logDenials?: boolean;
}

export function installPermissionHandlers(
  session: PermissionSession,
  { isAppWindow, logDenials = false }: PermissionHandlerOptions
) {
  const logged = new Set<string>();
  const decide = (
    contents: Electron.WebContents | null,
    permission: string,
    isMainFrame: boolean,
    requestingUrl?: string
  ) => {
    const appWindow = !!contents && isAppWindow(contents);
    const allowed = permissionAllowed({ permission, appWindow, isMainFrame, requestingUrl });
    // App windows carry their load settings in the query.
    const url = (requestingUrl || 'an unknown URL').split('?')[0];
    const key = `${permission} ${url}`;
    if (!allowed && logDenials && !logged.has(key)) {
      logged.add(key);
      console.warn(`Denied the "${permission}" permission to ${url}`);
    }
    return allowed;
  };

  session.setPermissionRequestHandler((contents, permission, callback, details) => {
    callback(decide(contents, permission, details.isMainFrame, details.requestingUrl));
  });
  session.setPermissionCheckHandler((contents, permission, requestingOrigin, details) =>
    decide(contents, permission, details.isMainFrame, details.requestingUrl || requestingOrigin)
  );
}
