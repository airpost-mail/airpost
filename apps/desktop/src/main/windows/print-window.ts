// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import fs from 'fs';
import path from 'path';
import { pathToFileURL } from 'url';
import { localized } from '../../intl/localize';
import { developerToolsAvailable } from '../../platform/developer-tools';
import { handleInvoke } from '../ipc/app-api';
import { PrintMessagesScriptName, PrintWindowOptions } from '../../platform/app-api-channels';

/*
The print preview window. The window that prints (features/print/window.ts) builds the page
and sends it over app-api:print:open; main writes it as print.html, with print.messages.js
next to it, into its own mkdtemp directory (0700, files 0600) and removes that when the window
closes, so on a shared /tmp nobody else can read the mail or plant the script the page loads.
features/print/static/print.js runs in it and asks for a PDF through the preload's
`printToPDF`, which posts 'print-to-pdf' to this window's own ipc.
*/

const PreloadPath = path.join(__dirname, '..', '..', 'features', 'print', 'static', 'preload.js');

/** The parts of a print window the module uses, so specs can pass fakes. */
export interface PrintWindowLike {
  webContents: {
    ipc: Pick<Electron.IpcMain, 'on'>;
    printToPDF(options: Electron.PrintToPDFOptions): Promise<Buffer>;
  };
  removeMenu(): void;
  loadURL(url: string): Promise<void>;
  once(event: 'closed', listener: () => void): void;
}

export interface PrintWindowHost {
  /** A new private directory for one print page. */
  makePrivateDir(): string;
  removeDir(dir: string): void;
  createWindow(options: Electron.BrowserWindowConstructorOptions): PrintWindowLike;
  showSaveDialog(options: Electron.SaveDialogOptions): Promise<Electron.SaveDialogReturnValue>;
  showMessageBox(options: Electron.MessageBoxOptions): Promise<Electron.MessageBoxReturnValue>;
  writeFile(filePath: string, data: Buffer): void;
  writePrivateFile(filePath: string, data: string): void;
}

export function createPrintWindowHost(): PrintWindowHost {
  const { app, BrowserWindow, dialog } = require('electron') as typeof Electron;
  return {
    makePrivateDir: () => fs.mkdtempSync(path.join(app.getPath('temp'), 'tern-print-')),
    removeDir: (dir) => fs.rmSync(dir, { recursive: true, force: true }),
    createWindow: (options) => new BrowserWindow(options),
    showSaveDialog: (options) => dialog.showSaveDialog(options),
    showMessageBox: (options) => dialog.showMessageBox(options),
    writeFile: (filePath, data) => fs.writeFileSync(filePath, data),
    writePrivateFile: (filePath, data) => fs.writeFileSync(filePath, data, { mode: 0o600 }),
  };
}

export function openPrintWindow(
  host: PrintWindowHost,
  { subject, html, messagesScript }: PrintWindowOptions
) {
  const dir = host.makePrivateDir();
  const pagePath = path.join(dir, 'print.html');
  host.writePrivateFile(path.join(dir, PrintMessagesScriptName), messagesScript);
  host.writePrivateFile(pagePath, html);

  const win = host.createWindow({
    width: 800,
    height: 600,
    title: `${localized('Print')} - ${subject}`,
    webPreferences: {
      preload: PreloadPath,
      nodeIntegration: false,
      contextIsolation: true,
      devTools: developerToolsAvailable(),
    },
  });

  win.webContents.ipc.on('print-to-pdf', async () => {
    const { filePath } = await host.showSaveDialog({ defaultPath: `${subject}.pdf` });
    if (!filePath) {
      return;
    }

    try {
      const data = await win.webContents.printToPDF({
        // Electron 21 replaced the old top-level `marginsType` enum with explicit
        // per-side inches (PrintToPDFMargins). `marginType: 'none'` was neither: an
        // unrecognised key, silently ignored, so these PDFs have been printing with the
        // default 1cm margins rather than none.
        margins: { top: 0, bottom: 0, left: 0, right: 0 },
        pageSize: 'Letter',
        printBackground: true,
        landscape: false,
      });
      host.writeFile(filePath, data);
    } catch (err) {
      // The dialog AppEnv.showErrorDialog shows.
      await host.showMessageBox({
        type: 'warning',
        buttons: [localized('Okay')],
        message: localized('Save as PDF Failed'),
        detail: localized(
          'Tern could not generate the PDF. Please try again. If the problem persists, try printing to a PDF printer instead.\n\nError: %@',
          err instanceof Error ? err.message : String(err)
        ),
      });
    }
  });

  win.once('closed', () => host.removeDir(dir));
  win.removeMenu();
  win
    .loadURL(pathToFileURL(pagePath).href)
    .catch((err: Error) => console.error('Print window failed to load:', err));
  return win;
}

export function registerPrintWindowIPC(
  ipcMain: Electron.IpcMain,
  host: PrintWindowHost,
  isTrustedSender: (contents: Electron.WebContents) => boolean
) {
  handleInvoke(ipcMain, { isTrustedSender }, 'app-api:print:open', (_event, options) => {
    openPrintWindow(host, {
      subject: String(options.subject),
      html: String(options.html),
      messagesScript: String(options.messagesScript),
    });
  });
}
