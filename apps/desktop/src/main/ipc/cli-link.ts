// SPDX-License-Identifier: GPL-3.0-only
import { app } from 'electron';
import { localized } from '../../intl/localize';
import { airpostBinaryPath } from '../../engine/process';
import { CliLinkChannel, CliLinkResult, CliLinkStatus } from '../../platform/cli-link';
import {
  installLink,
  linkLocations,
  linkStatus,
  osascriptAdminRunner,
  profileToPin,
  uninstallLink,
} from '../cli-link';
import { isAppWindowContents } from '../windows/app-window';

// Preferences > General > airpost command. Main finds the binary and the profile itself, so a
// window can't point the command (or the administrator prompt) anywhere else.
export function registerCliLinkIPC(
  ipcMain: Electron.IpcMain,
  resourcePath: string,
  configDirPath: string
) {
  const binary = () => airpostBinaryPath(resourcePath);
  const profile = profileToPin(configDirPath);
  const needsMove = () =>
    process.platform === 'darwin' && app.isPackaged && !app.isInApplicationsFolder();
  const status = (): CliLinkStatus => {
    const s = linkStatus(binary(), linkLocations(), { profile });
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
        profile,
        runAdmin: admin(localized('Airpost wants to install the airpost command.')),
      });
      return { ...result, status: status() };
    }
  );
  ipcMain.handle(CliLinkChannel.Uninstall, async (event): Promise<CliLinkResult | null> => {
    if (!isAppWindowContents(event.sender)) return null;
    const result = await uninstallLink(binary(), linkLocations(), {
      profile,
      runAdmin: admin(localized('Airpost wants to remove the airpost command.')),
    });
    return { ...result, status: status() };
  });
}
