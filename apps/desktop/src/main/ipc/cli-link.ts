// SPDX-License-Identifier: GPL-3.0-only
import { app } from 'electron';
import { localized } from '../../intl/localize';
import { ternBinaryPath } from '../../engine/process';
import { CliLinkChannel, CliLinkResult, CliLinkStatus } from '../../platform/cli-link';
import {
  installLink,
  linkLocations,
  linkStatus,
  osascriptAdminRunner,
  uninstallLink,
} from '../cli-link';
import { isAppWindowContents } from '../windows/app-window';

// Preferences > Agents > Command line. Main finds the binary itself, so a window can't point
// the link (or the administrator prompt) anywhere else.
export function registerCliLinkIPC(ipcMain: Electron.IpcMain, resourcePath: string) {
  const binary = () => ternBinaryPath(resourcePath);
  const needsMove = () =>
    process.platform === 'darwin' && app.isPackaged && !app.isInApplicationsFolder();
  const status = (): CliLinkStatus => {
    const s = linkStatus(binary(), linkLocations());
    return needsMove() ? { ...s, needsMove: true } : s;
  };
  const admin = (prompt: string) =>
    process.platform === 'darwin' ? osascriptAdminRunner(prompt) : null;

  ipcMain.handle(CliLinkChannel.Status, (event) => {
    if (!isAppWindowContents(event.sender)) return null;
    return status();
  });
  ipcMain.handle(
    CliLinkChannel.Install,
    async (event, replace: boolean): Promise<CliLinkResult | null> => {
      if (!isAppWindowContents(event.sender)) return null;
      if (needsMove()) return { ok: false, error: 'needs-move', status: status() };
      const result = await installLink(binary(), linkLocations(), {
        replace: !!replace,
        runAdmin: admin(localized('Tern wants to install the tern command.')),
      });
      return { ...result, status: status() };
    }
  );
  ipcMain.handle(CliLinkChannel.Uninstall, async (event): Promise<CliLinkResult | null> => {
    if (!isAppWindowContents(event.sender)) return null;
    const result = await uninstallLink(binary(), linkLocations(), {
      runAdmin: admin(localized('Tern wants to remove the tern command.')),
    });
    return { ...result, status: status() };
  });
}
