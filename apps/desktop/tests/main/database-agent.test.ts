// SPDX-License-Identifier: GPL-3.0-only
import { EventEmitter } from 'events';
import {
  AgentPath,
  DatabaseAgentHost,
  registerDatabaseAgentIPC,
} from '../../src/main/ipc/database-agent';

describe('the database agent IPC', function () {
  let onConnect: (event: any) => void;
  let agent: any;
  let host: DatabaseAgentHost;
  let contents: any;
  const port1: any = { name: 'port1' };
  const port2: any = { name: 'port2' };

  beforeEach(() => {
    agent = Object.assign(new EventEmitter(), {
      postMessage: jasmine.createSpy('postMessage'),
      kill: jasmine.createSpy('kill'),
      stdout: null,
      stderr: null,
    });
    host = {
      fork: jasmine.createSpy('fork').andReturn(agent),
      createChannel: () => ({ port1, port2 }),
    };
    contents = Object.assign(new EventEmitter(), {
      trusted: true,
      isDestroyed: () => false,
      send: jasmine.createSpy('send'),
      postMessage: jasmine.createSpy('postMessage'),
    });
    const ipcMain: any = { on: (_channel: string, fn: any) => (onConnect = fn) };
    registerDatabaseAgentIPC(ipcMain, host, (c: any) => !!c.trusted);
  });

  it('forks agent.js and hands the window and the agent each end of a channel', () => {
    onConnect({ sender: contents });
    expect((host.fork as jasmine.Spy).mostRecentCall.args[0]).toBe(AgentPath);
    expect(AgentPath).toMatch(/data[/\\]database[/\\]agent\.js$/);
    expect(agent.postMessage).toHaveBeenCalledWith(null, [port1]);
    const [channel, id, ports] = contents.postMessage.mostRecentCall.args;
    expect(channel).toBe('database-agent:port');
    expect(typeof id).toBe('number');
    expect(ports).toEqual([port2]);
  });

  it('tells the window when its agent exits, with the id of its port', () => {
    onConnect({ sender: contents });
    const id = contents.postMessage.mostRecentCall.args[1];
    agent.emit('exit', 0);
    expect(contents.send).toHaveBeenCalledWith('database-agent:exit', id);
  });

  it('kills the agent when the window goes away', () => {
    onConnect({ sender: contents });
    contents.emit('destroyed');
    expect(agent.kill).toHaveBeenCalled();
  });

  it('ignores windows that are not Tern windows', () => {
    contents.trusted = false;
    onConnect({ sender: contents });
    expect(host.fork).not.toHaveBeenCalled();
  });
});
