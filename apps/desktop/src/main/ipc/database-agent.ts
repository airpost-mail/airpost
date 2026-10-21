// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import path from 'path';

/*
The background query agent (data/database/agent.js) for DatabaseStore's `.background()`
queries. It runs as a utility process, one per window that asks, rather than through
child_process.fork from the window: fork runs the app binary with ELECTRON_RUN_AS_NODE, which
the RunAsNode fuse turns off in packaged builds (build/build.js). The window gets one end of a
MessageChannel ('database-agent:port') and the agent the other; when the agent exits, the window
is told ('database-agent:exit', with the id its port came with) so it can run what was in
flight itself. The agent exits on its own after a few idle seconds.
*/

let nextAgentId = 1;

export const AgentPath = path.join(__dirname, '..', '..', 'data', 'database', 'agent.js');

/** The parts of a utility process the module uses, so specs can pass fakes. */
export interface AgentProcessLike {
  postMessage(message: unknown, transfer?: Electron.MessagePortMain[]): void;
  once(event: 'exit', listener: (code: number) => void): void;
  kill(): boolean;
  stdout: NodeJS.ReadableStream | null;
  stderr: NodeJS.ReadableStream | null;
}

export interface DatabaseAgentHost {
  fork(modulePath: string): AgentProcessLike;
  createChannel(): { port1: Electron.MessagePortMain; port2: Electron.MessagePortMain };
}

export function createDatabaseAgentHost(): DatabaseAgentHost {
  const { utilityProcess, MessageChannelMain } = require('electron') as typeof Electron;
  return {
    fork: (modulePath) =>
      utilityProcess.fork(modulePath, [], { serviceName: 'Tern Database Agent', stdio: 'pipe' }),
    createChannel: () => new MessageChannelMain(),
  };
}

export function registerDatabaseAgentIPC(
  ipcMain: Pick<Electron.IpcMain, 'on'>,
  host: DatabaseAgentHost,
  isTrustedSender: (contents: Electron.WebContents) => boolean
) {
  ipcMain.on('database-agent:connect', (event) => {
    const contents = event.sender;
    if (!isTrustedSender(contents)) return;

    const id = nextAgentId++;
    const agent = host.fork(AgentPath);
    agent.stdout?.on('data', (data) => console.log(`Database agent: ${data}`));
    agent.stderr?.on('data', (data) => console.error(`Database agent: ${data}`));
    const kill = () => agent.kill();
    contents.once('destroyed', kill);
    agent.once('exit', () => {
      contents.removeListener('destroyed', kill);
      if (!contents.isDestroyed()) contents.send('database-agent:exit', id);
    });

    const { port1, port2 } = host.createChannel();
    agent.postMessage(null, [port1]);
    contents.postMessage('database-agent:port', id, [port2]);
  });
}
