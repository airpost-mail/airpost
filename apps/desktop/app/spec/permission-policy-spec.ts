import { installPermissionHandlers, permissionAllowed } from '../src/browser/permission-policy';

const AppURL = 'file:///Applications/Tern.app/Contents/Resources/app.asar/static/index.html';

describe('permissionAllowed', function () {
  const ask = (overrides = {}) => ({
    permission: 'notifications',
    appWindow: true,
    isMainFrame: true,
    requestingUrl: AppURL,
    ...overrides,
  });

  it('allows notifications and clipboard writes to the main frame of an app window', function () {
    expect(permissionAllowed(ask())).toBe(true);
    expect(permissionAllowed(ask({ permission: 'clipboard-sanitized-write' }))).toBe(true);
  });

  it('denies everything else, even to an app window', function () {
    for (const permission of [
      'media',
      'geolocation',
      'clipboard-read',
      'display-capture',
      'fullscreen',
      'openExternal',
      'pointerLock',
      'fileSystem',
      'unknown',
    ]) {
      expect(permissionAllowed(ask({ permission }))).toBe(false);
    }
  });

  it('denies frames inside an app window, such as message bodies', function () {
    expect(permissionAllowed(ask({ isMainFrame: false }))).toBe(false);
  });

  it('denies other windows (quick preview, print, show original, webviews)', function () {
    expect(permissionAllowed(ask({ appWindow: false }))).toBe(false);
  });

  it('allows fullscreen and clipboard writes to the pdf.js viewer main frame only', function () {
    const viewer = { appWindow: false, pdfViewer: true, requestingUrl: 'file:///x/viewer.html' };
    expect(permissionAllowed(ask({ ...viewer, permission: 'fullscreen' }))).toBe(true);
    expect(permissionAllowed(ask({ ...viewer, permission: 'clipboard-sanitized-write' }))).toBe(
      true
    );
    expect(permissionAllowed(ask({ ...viewer, permission: 'notifications' }))).toBe(false);
    expect(permissionAllowed(ask({ ...viewer, permission: 'clipboard-read' }))).toBe(false);
    expect(
      permissionAllowed(ask({ ...viewer, permission: 'fullscreen', isMainFrame: false }))
    ).toBe(false);
    expect(permissionAllowed(ask({ appWindow: false, permission: 'fullscreen' }))).toBe(false);
  });

  it('denies an app window that is not showing the app', function () {
    expect(permissionAllowed(ask({ requestingUrl: 'https://example.com/' }))).toBe(false);
    expect(permissionAllowed(ask({ requestingUrl: undefined }))).toBe(false);
  });
});

describe('installPermissionHandlers', function () {
  let request: (...args: any[]) => void;
  let check: (...args: any[]) => boolean;
  const appContents = { id: 1 } as any;
  const otherContents = { id: 2 } as any;

  const install = (logDenials = false) => {
    installPermissionHandlers(
      {
        setPermissionRequestHandler: (handler) => {
          request = handler;
        },
        setPermissionCheckHandler: (handler) => {
          check = handler;
        },
      },
      { isAppWindow: (contents) => contents === appContents, logDenials }
    );
  };

  beforeEach(function () {
    spyOn(console, 'warn');
  });

  it('answers requests', function () {
    install();
    const answers = [];
    const answer = (granted) => answers.push(granted);
    request(appContents, 'notifications', answer, { isMainFrame: true, requestingUrl: AppURL });
    request(appContents, 'media', answer, { isMainFrame: true, requestingUrl: AppURL });
    request(otherContents, 'notifications', answer, { isMainFrame: true, requestingUrl: AppURL });
    expect(answers).toEqual([true, false, false]);
  });

  it('answers checks, falling back to the origin and denying without a webContents', function () {
    install();
    expect(check(appContents, 'clipboard-sanitized-write', 'file:///', { isMainFrame: true })).toBe(
      true
    );
    expect(check(appContents, 'geolocation', 'file:///', { isMainFrame: true })).toBe(false);
    expect(check(null, 'notifications', 'file:///', { isMainFrame: true })).toBe(false);
  });

  it('logs each denial once in dev mode only', function () {
    install(true);
    const details = { isMainFrame: true, requestingUrl: AppURL };
    request(appContents, 'media', () => {}, details);
    request(appContents, 'media', () => {}, details);
    request(appContents, 'notifications', () => {}, details);
    expect((console.warn as jasmine.Spy).callCount).toBe(1);

    install(false);
    request(appContents, 'geolocation', () => {}, details);
    expect((console.warn as jasmine.Spy).callCount).toBe(1);
  });
});

describe('isPdfViewerURL', function () {
  const { isPdfViewerURL } = require('../src/browser/quickpreview-ipc');
  const path = require('path');
  const { pathToFileURL } = require('url');
  const viewer = path.join(
    __dirname,
    '..',
    'src',
    'quickpreview',
    'pdfjs-4.3.136',
    'web',
    'viewer.html'
  );

  it('matches the bundled viewer with its query, and nothing else', function () {
    const url = pathToFileURL(viewer).href;
    expect(isPdfViewerURL(`${url}?file=${encodeURIComponent('file:///tmp/a.pdf')}`)).toBe(true);
    expect(isPdfViewerURL(url.replace('viewer.html', 'other.html'))).toBe(false);
    expect(isPdfViewerURL('file:///tmp/web/viewer.html')).toBe(false);
    expect(isPdfViewerURL(url.replace('file:///', 'file://host/'))).toBe(false);
    expect(isPdfViewerURL('https://example.com/web/viewer.html')).toBe(false);
    expect(isPdfViewerURL(undefined)).toBe(false);
  });
});
