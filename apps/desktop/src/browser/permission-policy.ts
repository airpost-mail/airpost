// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Web permissions, denied unless asked for by the main frame of one of Tern's own windows, and then
only the ones the app uses. Without a handler Electron grants nearly everything (camera,
microphone, geolocation, clipboard-read, screen capture) to any frame, message iframes included.

- clipboard-sanitized-write: navigator.clipboard.writeText (CopyButton, thread sharing).
- notifications: the web Notification API, for plugins. Tern's own notifications are shown from
  main (notification-ipc.ts) and need no permission.

The quick preview window is not one of them, but shows PDFs in the bundled pdf.js viewer, whose
main frame gets fullscreen (Presentation Mode) and clipboard-sanitized-write (copying a
select-all).

Sounds (SoundRegistry) play through an Audio element, which needs no permission; they do need
the autoplay-policy switch in main.js, since a new-mail sound plays with no user gesture.
*/

export const AppWindowPermissions: ReadonlySet<string> = new Set([
  'clipboard-sanitized-write',
  'notifications',
]);

export const PdfViewerPermissions: ReadonlySet<string> = new Set([
  'clipboard-sanitized-write',
  'fullscreen',
]);

export interface PermissionAsk {
  permission: string;
  // The asking webContents is one of Tern's windows (isAppWindowContents).
  appWindow: boolean;
  // The asking frame shows the bundled pdf.js viewer (isPdfViewerURL).
  pdfViewer?: boolean;
  isMainFrame: boolean;
  requestingUrl?: string;
}

export function permissionAllowed({
  permission,
  appWindow,
  pdfViewer = false,
  isMainFrame,
  requestingUrl,
}: PermissionAsk) {
  if (!isMainFrame || !/^file:/i.test(requestingUrl || '')) return false;
  if (pdfViewer && PdfViewerPermissions.has(permission)) return true;
  return appWindow && AppWindowPermissions.has(permission);
}

type PermissionSession = Pick<
  Electron.Session,
  'setPermissionRequestHandler' | 'setPermissionCheckHandler'
>;

export interface PermissionHandlerOptions {
  isAppWindow: (contents: Electron.WebContents) => boolean;
  isPdfViewer?: (url: string | undefined) => boolean;
  // Dev mode: each denied permission and URL is logged once.
  logDenials?: boolean;
}

export function installPermissionHandlers(
  session: PermissionSession,
  { isAppWindow, isPdfViewer = () => false, logDenials = false }: PermissionHandlerOptions
) {
  const logged = new Set<string>();
  const decide = (
    contents: Electron.WebContents | null,
    permission: string,
    isMainFrame: boolean,
    requestingUrl?: string
  ) => {
    const appWindow = !!contents && isAppWindow(contents);
    const pdfViewer = !!contents && isPdfViewer(requestingUrl);
    const allowed = permissionAllowed({
      permission,
      appWindow,
      pdfViewer,
      isMainFrame,
      requestingUrl,
    });
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
