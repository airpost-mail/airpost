// SPDX-License-Identifier: GPL-3.0-only
import {
  EngineUnitResult,
  GatewayPairing,
  ServicesChannel,
  type EngineUnitStatus,
} from '../../platform/services';
import type { LocalServices } from '../services';
import { engineUnitStatus, setEngineUnit } from '../services/engine-unit';
import { isAppWindowContents } from '../windows/app-window';

// Preferences > General > Services on this computer, and Preferences > MCP > HTTP. Main picks
// the binaries, profile, ports and arguments; a window only reads state and asks for the
// actions, so it can't make main run anything else.
export function registerServicesIPC(
  ipcMain: Electron.IpcMain,
  services: LocalServices | null,
  {
    configDirPath,
    resourcePath,
    joinedEngine,
  }: { configDirPath: string; resourcePath: string; joinedEngine: () => boolean }
) {
  const unitStatus = (): EngineUnitStatus => {
    const status = engineUnitStatus(configDirPath, resourcePath, { joined: false });
    return { ...status, joined: status.running && joinedEngine() };
  };

  ipcMain.handle(ServicesChannel.McpHttpStatus, (event) => {
    if (!isAppWindowContents(event.sender) || !services) return null;
    return services.mcpHttp.status();
  });
  ipcMain.handle(ServicesChannel.McpHttpReload, (event) => {
    if (!isAppWindowContents(event.sender) || !services) return null;
    return services.mcpHttp.reload();
  });
  ipcMain.handle(ServicesChannel.GatewayStatus, (event) => {
    if (!isAppWindowContents(event.sender) || !services) return null;
    return services.gateway.status();
  });
  ipcMain.handle(ServicesChannel.GatewayPair, async (event): Promise<GatewayPairing | null> => {
    if (!isAppWindowContents(event.sender) || !services) return null;
    if (services.gateway.status().state !== 'running') {
      return { ok: false, error: 'not-running' };
    }
    return services.gateway.pair();
  });
  ipcMain.handle(ServicesChannel.EngineUnitStatus, (event) => {
    if (!isAppWindowContents(event.sender)) return null;
    return unitStatus();
  });
  ipcMain.handle(
    ServicesChannel.EngineUnitSet,
    async (event, install: boolean): Promise<EngineUnitResult | null> => {
      if (!isAppWindowContents(event.sender)) return null;
      const result = await setEngineUnit(configDirPath, resourcePath, !!install, {
        joined: joinedEngine(),
      });
      return { ...result, status: unitStatus() };
    }
  );
}
