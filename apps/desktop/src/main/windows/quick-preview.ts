// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import path from 'path';
import { BrowserWindow, Menu } from 'electron';
import { developerToolsAvailable } from '../../platform/developer-tools';
import { quickPreviewContentSecurityPolicy } from '../../mail/content-security-policy';
import { isFileURLWithin } from '../../utils/file-url';
import { cleanupPreviewToken, generatePreviewToken } from '../ipc/quick-preview';
import { handleInvoke } from '../ipc/app-api';
import type { QuickPreviewCapture, QuickPreviewDisplay } from '../../platform/app-api-channels';

/*
The attachment quick preview windows: the preview window the user opens, and the hidden capture
window that renders thumbnails one at a time. Windows ask for them over app-api channels
(mail/quick-preview/index.ts picks the strategy); mail/quick-preview/renderer.html runs in them
and talks back over the quickpreview: channels in ipc/quick-preview.ts.
*/

// A dedicated in-memory session partition, so the strict CSP below can be applied with
// onHeadersReceived without replacing the default session's listener (main.js), which injects
// the app-wide CSP. Electron keeps one onHeadersReceived listener per session.
const QuickPreviewPartition = 'quickpreview';

const filesRoot = path
  .join(__dirname, '..', '..', 'mail', 'quick-preview')
  .replace('app.asar', 'app.asar.unpacked');
// pdf.js, Prism, SheetJS and DOMPurify, which renderer.html loads by relative path.
const vendorRoot = path.resolve(filesRoot, '..', '..', '..', 'vendor');
const PDFJSRoot = path.join(vendorRoot, 'pdfjs-4.3.136');

const ThumbnailWidth = 320 * (11 / 8.5);
// Budget for generating a single thumbnail.
const PreviewTimeout = 5000;

const Strategies = ['pdfjs', 'mammoth', 'snarkdown', 'xlsx', 'prism'];

interface CaptureJob extends QuickPreviewCapture {
  resolve: (success: boolean) => void;
}

export interface QuickPreviewHost {
  configDirPath: string;
  autoloadImages(): boolean;
}

let host: QuickPreviewHost | null = null;
let quickPreviewWindow: BrowserWindow | null = null;
let quickPreviewWebContentsId: number | null = null;
let captureWindow: BrowserWindow | null = null;
let captureWindowInUse = false;
const captureQueue: CaptureJob[] = [];

// Both windows share the partition and so one listener. Thumbnails never load remote images;
// the preview window does when "load images automatically" is on. Attachments carry no sender
// here, so the per-sender allowlist doesn't apply.
//
// 'self' in the policy matches every file: URL, so file: requests are also limited to the
// bundled preview code and downloaded attachments. That keeps attachment content (markdown or
// docx with <img src="//host/share/x">) off UNC paths, which Windows opens over SMB.
function applyQuickPreviewCSP(session: Electron.Session) {
  if (!host) return;
  const { configDirPath, autoloadImages } = host;
  const fileRoots = [filesRoot, vendorRoot, path.join(configDirPath, 'files')];
  session.webRequest.onBeforeRequest((details, callback) => {
    const isFile = /^file:/i.test(details.url);
    callback({ cancel: isFile && !isFileURLWithin(details.url, fileRoots) });
  });
  session.webRequest.onHeadersReceived((details, callback) => {
    const remoteImages = details.webContentsId === quickPreviewWebContentsId && autoloadImages();
    callback({
      responseHeaders: {
        ...details.responseHeaders,
        'Content-Security-Policy': [quickPreviewContentSecurityPolicy({ remoteImages })],
      },
    });
  });
}

function previewWindowMenuTemplate(): Electron.MenuItemConstructorOptions[] {
  return [
    {
      label: 'File',
      role: 'window',
      submenu: [
        { label: 'Minimize Window', accelerator: 'CmdOrCtrl+M', role: 'minimize' },
        { label: 'Close Window', accelerator: 'CmdOrCtrl+W', role: 'close' },
      ],
    },
    {
      label: 'Edit',
      submenu: [
        { label: 'Undo', accelerator: 'CmdOrCtrl+Z', role: 'undo' },
        { label: 'Redo', accelerator: 'Shift+CmdOrCtrl+Z', role: 'redo' },
        { type: 'separator' },
        { label: 'Cut', accelerator: 'CmdOrCtrl+X', role: 'cut' },
        { label: 'Copy', accelerator: 'CmdOrCtrl+C', role: 'copy' },
        { label: 'Paste', accelerator: 'CmdOrCtrl+V', role: 'paste' },
        { label: 'Select All', accelerator: 'CmdOrCtrl+A', role: 'selectAll' },
      ],
    },
    {
      label: 'View',
      submenu: [
        {
          label: 'Reload',
          accelerator: 'CmdOrCtrl+R',
          click: (_item, focusedWindow) => {
            if (focusedWindow instanceof BrowserWindow) focusedWindow.reload();
          },
        },
        {
          label: 'Toggle Full Screen',
          accelerator: process.platform === 'darwin' ? 'Ctrl+Command+F' : 'F11',
          click: (_item, focusedWindow) => {
            if (focusedWindow) focusedWindow.setFullScreen(!focusedWindow.isFullScreen());
          },
        },
        ...(developerToolsAvailable()
          ? [
              {
                label: 'Toggle Developer Tools',
                accelerator: process.platform === 'darwin' ? 'Alt+Command+I' : 'Ctrl+Shift+I',
                click: (_item: Electron.MenuItem, focusedWindow?: Electron.BaseWindow) => {
                  if (focusedWindow instanceof BrowserWindow) {
                    focusedWindow.webContents.toggleDevTools();
                  }
                },
              },
            ]
          : []),
      ],
    },
  ];
}

export function displayQuickPreviewWindow({ filePath, strategy }: QuickPreviewDisplay) {
  const isPDF = filePath.endsWith('.pdf');

  let previewWindow = quickPreviewWindow;
  if (previewWindow === null) {
    const win = new BrowserWindow({
      width: 800,
      height: 600,
      center: true,
      skipTaskbar: true,
      backgroundColor: isPDF ? '#404040' : '#FFF',
      webPreferences: {
        preload: path.join(filesRoot, 'preload.js'),
        nodeIntegration: false,
        contextIsolation: true,
        sandbox: true,
        partition: QuickPreviewPartition,
        devTools: developerToolsAvailable(),
      },
    });
    previewWindow = win;
    quickPreviewWindow = win;
    quickPreviewWebContentsId = win.webContents.id;
    applyQuickPreviewCSP(win.webContents.session);

    win.once('closed', () => {
      quickPreviewWindow = null;
      quickPreviewWebContentsId = null;
    });
    win.setMenu(Menu.buildFromTemplate(previewWindowMenuTemplate()));
  } else {
    previewWindow.show();
  }
  previewWindow.setTitle(path.basename(filePath));

  const onLoadError = (err: Error) => {
    if (!err?.message?.includes('ERR_ABORTED')) {
      console.error('Quickpreview display window failed to load:', err);
    }
  };

  if (isPDF) {
    previewWindow
      .loadFile(path.join(PDFJSRoot, 'web/viewer.html'), {
        search: `file=${encodeURIComponent(`file://${filePath}`)}`,
      })
      .catch(onLoadError);
  } else {
    previewWindow
      .loadFile(path.join(filesRoot, 'renderer.html'), {
        search: JSON.stringify({ mode: 'display', filePath, strategy }),
      })
      .catch(onLoadError);
  }
}

export function capturePreview(job: QuickPreviewCapture) {
  return new Promise<boolean>((resolve) => {
    captureQueue.push({ ...job, resolve });

    if (!captureWindow || captureWindow.isDestroyed()) {
      captureWindow = createCaptureWindow();
      generateNextPreview();
    }
  });
}

function createCaptureWindow() {
  const win = new BrowserWindow({
    width: ThumbnailWidth,
    height: ThumbnailWidth,
    show: false,
    webPreferences: {
      preload: path.join(filesRoot, 'preload.js'),
      nodeIntegration: false,
      contextIsolation: true,
      sandbox: true,
      partition: QuickPreviewPartition,
      devTools: developerToolsAvailable(),
    },
  });

  applyQuickPreviewCSP(win.webContents.session);

  win.webContents.on('render-process-gone', (_event, details) => {
    console.warn(`Thumbnail generation webcontents crashed (reason: ${details.reason}).`);
    if (captureWindow === win) captureWindow = null;
    win.destroy();
  });
  win.once('closed', () => {
    if (captureWindow === win) captureWindow = null;
  });
  return win;
}

function generateNextPreview() {
  const job = captureQueue.pop();
  if (!job) {
    // Don't tear down the window while a generation is in progress: that one schedules the
    // next call once it finishes, which then reaches this branch and cleans up.
    if (!captureWindowInUse) {
      if (captureWindow && !captureWindow.isDestroyed()) {
        captureWindow.destroy();
      } else {
        console.warn(`Thumbnail generation finished but window is already destroyed.`);
      }
      captureWindow = null;
    }
    return;
  }

  const { strategy, filePath, previewPath, resolve } = job;
  captureWindowInUse = true;

  // The renderer is only given an opaque token for the path it may write.
  const previewToken = generatePreviewToken(previewPath);

  let win = captureWindow;
  if (!win || win.isDestroyed()) {
    win = createCaptureWindow();
    captureWindow = win;
  }

  win
    .loadFile(path.join(filesRoot, 'renderer.html'), {
      search: JSON.stringify({ strategy, mode: 'capture', filePath, previewToken }),
    })
    .catch((err: Error) => {
      // ERR_ABORTED is expected when a new preview starts while the previous is still loading
      // (e.g. after the timeout fires). The thumbnail just won't be generated.
      if (!err?.message?.includes('ERR_ABORTED')) {
        console.error('Quickpreview capture window failed to load:', err);
      }
    });

  // Race against a timer so a renderer exception can't hang the queue.
  const timer = setTimeout(() => {
    console.warn(`Thumbnail generation timed out for ${filePath}`);
    onFinalize(false);
  }, PreviewTimeout);

  const onRendererSuccess = () => {
    onFinalize(true);
  };

  function onFinalize(success: boolean) {
    captureWindowInUse = false;
    clearTimeout(timer);
    if (captureWindow) {
      captureWindow.removeListener('page-title-updated', onRendererSuccess);
    }
    // On success the finishCapture/finishWithData handler has used up the token.
    if (!success) {
      cleanupPreviewToken(previewToken);
    }
    process.nextTick(generateNextPreview);
    resolve(success);
  }

  win.once('page-title-updated', onRendererSuccess);
}

export function registerQuickPreviewWindowsIPC(
  ipcMain: Electron.IpcMain,
  previewHost: QuickPreviewHost,
  isTrustedSender: (contents: Electron.WebContents) => boolean
) {
  host = previewHost;
  const trusted = { isTrustedSender };
  handleInvoke(ipcMain, trusted, 'app-api:quickpreview:display', (_event, options) => {
    if (options.strategy !== undefined && !Strategies.includes(options.strategy)) {
      throw new Error(`Unknown quick preview strategy ${options.strategy}`);
    }
    displayQuickPreviewWindow(options);
  });
  handleInvoke(ipcMain, trusted, 'app-api:quickpreview:capture', (_event, options) => {
    if (!Strategies.includes(options.strategy)) {
      throw new Error(`Unknown quick preview strategy ${options.strategy}`);
    }
    return capturePreview(options);
  });
}
