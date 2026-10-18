import { BrowserWindow, dialog } from 'electron';
import { localized } from '../intl';
import { ConfirmOpenChannel } from '../attachment-safety';
import { isMailspringWindowContents } from './mailspring-window';

// The "open this executable?" prompt for src/attachment-safety.ts. Cancel is the default.
export function registerAttachmentSafetyIPC(ipcMain: Electron.IpcMain) {
  ipcMain.handle(ConfirmOpenChannel, async (event, message: string, detail: string) => {
    if (!isMailspringWindowContents(event.sender)) {
      return false;
    }
    const options: Electron.MessageBoxOptions = {
      type: 'warning',
      buttons: [localized('Cancel'), localized('Open')],
      defaultId: 0,
      cancelId: 0,
      message: String(message),
      detail: String(detail),
    };
    const win = BrowserWindow.fromWebContents(event.sender);
    const { response } = win
      ? await dialog.showMessageBox(win, options)
      : await dialog.showMessageBox(options);
    return response === 1;
  });
}
