// SPDX-License-Identifier: GPL-3.0-only
import type Config from '../../config/config';
import { ternGatewayBinaryPath, ternMcpBinaryPath, ternWebRootPath } from '../../engine/process';
import { ServicesChannel } from '../../platform/services';
import { GatewayService } from './gateway';
import { McpHttpService } from './mcp-http';

export const WEB_ACCESS_KEY = 'core.services.webAccess';

/** The servers main runs while the app runs: MCP over HTTP and web access. */
export class LocalServices {
  readonly mcpHttp: McpHttpService;
  readonly gateway: GatewayService;

  constructor(
    private opts: {
      configDirPath: string;
      resourcePath: string;
      config: Config;
      broadcast: (channel: string, payload: unknown) => void;
    }
  ) {
    const { configDirPath, resourcePath, broadcast } = opts;
    this.mcpHttp = new McpHttpService(configDirPath, () => ternMcpBinaryPath(resourcePath));
    this.gateway = new GatewayService(
      configDirPath,
      () => ternGatewayBinaryPath(resourcePath),
      () => ternWebRootPath(resourcePath)
    );
    this.mcpHttp.on('change', (s) => broadcast(ServicesChannel.Changed, s));
    this.gateway.on('change', (s) => broadcast(ServicesChannel.Changed, s));
  }

  start() {
    const { config } = this.opts;
    this.mcpHttp.start();
    this.gateway.setEnabled(!!config.get(WEB_ACCESS_KEY));
    config.onDidChange(WEB_ACCESS_KEY, ({ newValue }) => {
      this.gateway.setEnabled(!!newValue);
    });
  }

  /** At quit: SIGTERM now, without waiting, so the ports are free for the next launch. */
  terminate() {
    this.mcpHttp.stop();
    this.gateway.stop();
    this.mcpHttp.child.terminate();
    this.gateway.child.terminate();
  }
}
