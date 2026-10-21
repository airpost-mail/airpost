// SPDX-License-Identifier: GPL-3.0-only
import fs from 'fs';
import os from 'os';
import path from 'path';
import { SupervisedChild } from '../../src/main/services/supervised-child';
import { McpHttpService, mcpReadyLine, readHttpPolicy } from '../../src/main/services/mcp-http';
import { GatewayService, gatewayReadyLine } from '../../src/main/services/gateway';
import { Runner, engineUnitStatus, setEngineUnit } from '../../src/main/services/engine-unit';

// The children here are small sh scripts standing in for tern-mcp and tern-gateway.

async function until(check: () => boolean, timeoutMs = 5000) {
  const start = Date.now();
  while (!check()) {
    if (Date.now() - start > timeoutMs) throw new Error('timed out');
    await new Promise((r) => setTimeout(r, 20));
  }
}

function script(dir: string, name: string, body: string) {
  const file = path.join(dir, name);
  fs.writeFileSync(file, `#!/bin/sh\n${body}\n`, { mode: 0o755 });
  return file;
}

describe('the services main runs', function () {
  let dir: string;
  const children: { stop: () => Promise<void> }[] = [];

  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tern-services-'));
    // Real timers: the children are real processes, and the restart delays real waits.
    jasmine.unspy(window, 'setTimeout');
    jasmine.unspy(window, 'clearTimeout');
  });

  afterEach(async () => {
    await Promise.all(children.splice(0).map((c) => c.stop()));
    fs.rmSync(dir, { recursive: true, force: true });
  });

  describe('SupervisedChild', () => {
    const ready = (line: string) => (line.startsWith('ready ') ? { url: line.slice(6) } : null);

    function supervise(opts = {}) {
      const child = new SupervisedChild({
        name: 'test',
        ready,
        initialDelayMs: 50,
        stopTimeoutMs: 500,
        ...opts,
      });
      children.push(child);
      return child;
    }

    it('reports running once the child says so, and stops it', async () => {
      const bin = script(dir, 'server', 'echo "ready http://x/$1" >&2; exec sleep 30');
      const child = supervise();
      await child.setSpec({ command: bin, args: ['a'] });
      await until(() => child.status().state === 'running');
      expect(child.status().url).toBe('http://x/a');
      const pid = child.status().pid as number;
      expect(() => process.kill(pid, 0)).not.toThrow();

      await child.stop();
      expect(child.status().state).toBe('stopped');
      await until(() => {
        try {
          process.kill(pid, 0);
          return false;
        } catch (err) {
          return true;
        }
      });
    });

    it('restarts a child that exits, with a growing delay, and shows why it exited', async () => {
      const count = path.join(dir, 'count');
      const bin = script(dir, 'crash', `echo x >> '${count}'; echo "port in use" >&2; exit 3`);
      spyOn(console, 'warn');
      const child = supervise();
      await child.setSpec({ command: bin, args: [] });
      await until(() => fs.existsSync(count) && fs.readFileSync(count, 'utf8').length >= 6);
      expect(child.status().state).toBe('error');
      expect(child.status().error).toBe('port in use');
      await child.stop();
      const runs = fs.readFileSync(count, 'utf8').length;
      await new Promise((r) => setTimeout(r, 300));
      expect(fs.readFileSync(count, 'utf8').length).toBe(runs);
    });

    it('restarts at once when the spec changes, and leaves it alone when it does not', async () => {
      const bin = script(dir, 'server', 'echo "ready $1" >&2; exec sleep 30');
      const child = supervise();
      await child.setSpec({ command: bin, args: ['1'] });
      await until(() => child.status().state === 'running');
      const first = child.status().pid;
      await child.setSpec({ command: bin, args: ['1'] });
      expect(child.status().pid).toBe(first);
      await child.setSpec({ command: bin, args: ['2'] });
      await until(() => child.status().url === '2');
      expect(child.status().pid).not.toBe(first);
    });

    it('reports a binary it cannot start', async () => {
      spyOn(console, 'warn');
      const child = supervise({ initialDelayMs: 10000 });
      await child.setSpec({ command: path.join(dir, 'missing'), args: [] });
      await until(() => child.status().state === 'error');
      expect(child.status().error).toContain('ENOENT');
    });
  });

  it('reads mcp.json’s http as tern-mcp does', () => {
    const write = (v: string) => fs.writeFileSync(path.join(dir, 'mcp.json'), v);
    expect(readHttpPolicy(dir)).toEqual({ enabled: false, port: 2587, error: null });
    write('{"http": {"enabled": true, "port": 3000}}');
    expect(readHttpPolicy(dir)).toEqual({ enabled: true, port: 3000, error: null });
    write('{"enabled": false, "http": {"enabled": true}}');
    expect(readHttpPolicy(dir)).toEqual({ enabled: true, port: 2587, error: null });
    write('{"http": {"enabled": true, "port": 70000}}');
    expect(readHttpPolicy(dir).enabled).toBe(false);
    write('{"http": ');
    expect(readHttpPolicy(dir).enabled).toBe(false);
    expect(readHttpPolicy(dir).error).toBeTruthy();
  });

  it('finds the URL in the servers’ ready lines', () => {
    expect(mcpReadyLine('[tern-mcp] listening on http://127.0.0.1:2587/mcp')).toEqual({
      url: 'http://127.0.0.1:2587/mcp',
    });
    expect(mcpReadyLine('[tern-mcp] HTTP is off')).toBe(null);
    expect(
      gatewayReadyLine(
        '2026-10-21T10:00:00Z  INFO tern_gateway::cli: tern-gateway serving listen=127.0.0.1:5180 public_url=http://127.0.0.1:5180'
      )
    ).toEqual({ url: 'http://127.0.0.1:5180' });
    expect(gatewayReadyLine('INFO shutting down')).toBe(null);
  });

  it('runs tern-mcp http while mcp.json turns HTTP on, on its port', async () => {
    const args = path.join(dir, 'args');
    const bin = script(
      dir,
      'tern-mcp',
      `echo "$@" > '${args}'; echo "[tern-mcp] listening on http://127.0.0.1:$5/mcp" >&2; exec sleep 30`
    );
    const write = (http: object) =>
      fs.writeFileSync(path.join(dir, 'mcp.json'), JSON.stringify({ http }));
    const service = new McpHttpService(dir, () => bin);
    children.push(service);

    expect((await service.start()).state).toBe('stopped');
    write({ enabled: true, port: 3001 });
    await service.reload();
    await until(() => service.status().state === 'running');
    expect(service.status().url).toBe('http://127.0.0.1:3001/mcp');
    expect(fs.readFileSync(args, 'utf8').trim()).toBe(`--config-dir ${dir} http --port 3001`);
    expect(service.status().tokenFile).toBe(path.join(dir, 'mcp-token'));

    write({ enabled: true, port: 3002 });
    await service.reload();
    await until(() => service.status().url === 'http://127.0.0.1:3002/mcp');

    write({ enabled: false, port: 3002 });
    await service.reload();
    expect(service.status().state).toBe('stopped');
    expect(service.status().enabled).toBe(false);
  });

  it('runs tern-gateway with the web client, and pairs a browser', async () => {
    const web = path.join(dir, 'web');
    fs.mkdirSync(web);
    const args = path.join(dir, 'args');
    const bin = script(
      dir,
      'tern-gateway',
      [
        `if [ "$3" = pair ]; then echo '{"code":"ABCD-1234","url":"http://127.0.0.1:5180/pair#ABCD-1234","admin":false,"expiresAt":1}'; exit 0; fi`,
        `echo "$@" > '${args}'`,
        `echo "INFO tern_gateway::cli: tern-gateway serving listen=127.0.0.1:5180 public_url=http://127.0.0.1:5180" >&2`,
        'exec sleep 30',
      ].join('\n')
    );
    let webRoot: string | null = null;
    const service = new GatewayService(
      dir,
      () => bin,
      () => webRoot
    );
    children.push(service);

    await service.setEnabled(true);
    expect(service.status().state).toBe('stopped');
    expect(service.status().available).toBe(false);
    expect(service.status().missing).toEqual(['web']);
    webRoot = web;
    await service.setEnabled(true);
    await until(() => service.status().state === 'running');
    expect(service.status().url).toBe('http://127.0.0.1:5180');
    expect(fs.readFileSync(args, 'utf8').trim()).toBe(
      `--config-dir ${dir} serve --bind 127.0.0.1:5180 --web-root ${web}`
    );
    expect(await service.pair()).toEqual({
      ok: true,
      code: 'ABCD-1234',
      url: 'http://127.0.0.1:5180/pair#ABCD-1234',
      expiresAt: 1,
    });
    await service.setEnabled(false);
    expect(service.status().state).toBe('stopped');
  });

  describe('the engine unit', () => {
    let resources: string;
    let saved: { [key: string]: string | undefined };

    beforeEach(() => {
      saved = { TERN_BIN: process.env.TERN_BIN, KEY: process.env.TERN_ENGINE_KEY_FILE };
      delete process.env.TERN_BIN;
      delete process.env.TERN_ENGINE_KEY_FILE;
      resources = path.join(dir, 'resources');
      fs.mkdirSync(resources);
      fs.writeFileSync(path.join(resources, 'tern'), '');
    });

    afterEach(() => {
      for (const [key, env] of [
        ['TERN_BIN', saved.TERN_BIN],
        ['TERN_ENGINE_KEY_FILE', saved.KEY],
      ]) {
        if (env === undefined) delete process.env[key as string];
        else process.env[key as string] = env;
      }
    });

    /** Records the commands and writes services.json as `tern engine install` does. */
    function fakeTern(calls: string[][], { fail = '' } = {}): Runner {
      return async (file, args) => {
        calls.push([path.basename(file), ...args]);
        if (fail) return { ok: false, stdout: '', stderr: `error: ${fail}\n` };
        if (args.includes('install')) {
          const unit = path.join(dir, 'tern-engine.service');
          fs.writeFileSync(unit, '');
          fs.writeFileSync(
            path.join(dir, 'services.json'),
            JSON.stringify({ engine: { name: 'tern-engine', unit } })
          );
        }
        return { ok: true, stdout: '', stderr: '' };
      };
    }

    it('installs the unit without loading it, with the dev key file, and enables it on Linux', async () => {
      const calls: string[][] = [];
      expect(engineUnitStatus(dir, resources, { joined: false, platform: 'linux' })).toEqual({
        available: true,
        installed: false,
        running: false,
        joined: false,
        unit: null,
      });
      const result = await setEngineUnit(dir, resources, true, {
        joined: false,
        platform: 'linux',
        run: fakeTern(calls),
      });
      expect(result.ok).toBe(true);
      expect(result.status.installed).toBe(true);
      expect(calls).toEqual([
        [
          'tern',
          '--config-dir',
          dir,
          'engine',
          'install',
          '--no-load',
          '--key-file',
          path.join(dir, 'engine-dev.key'),
        ],
        ['systemctl', '--user', 'daemon-reload'],
        ['systemctl', '--user', 'enable', 'tern-engine.service'],
      ]);
    });

    it('uninstalls through tern and passes its error on', async () => {
      const calls: string[][] = [];
      const refused = await setEngineUnit(dir, resources, false, {
        joined: false,
        platform: 'darwin',
        run: fakeTern(calls, {
          fail: 'these services run on the engine unit; uninstall them first: x',
        }),
      });
      expect(refused.ok).toBe(false);
      expect(refused.error).toBe('these services run on the engine unit; uninstall them first: x');
      expect(calls).toEqual([['tern', '--config-dir', dir, 'engine', 'uninstall']]);
    });

    it('is unavailable without tern or off macOS and Linux', async () => {
      expect(engineUnitStatus(dir, resources, { joined: false, platform: 'win32' }).available).toBe(
        false
      );
      fs.rmSync(path.join(resources, 'tern'));
      const result = await setEngineUnit(dir, resources, true, {
        joined: false,
        platform: 'darwin',
        run: fakeTern([]),
      });
      expect(result.ok).toBe(false);
      expect(result.error).toBe('unsupported');
    });
  });
});
