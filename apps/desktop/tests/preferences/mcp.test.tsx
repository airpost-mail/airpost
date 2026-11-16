import fs from 'fs';
import os from 'os';
import path from 'path';
import React from 'react';
import ReactDOM from 'react-dom';
import TestRenderer, { act } from 'react-test-renderer';
import { AccountStore } from '../../src/data/stores/account';
import CategoryStore from '../../src/data/stores/category';
import { SettingsSwitch } from '../../src/components/settings';
import { Button } from '../../src/components/kit/button';

import { McpSection } from '../../src/features/preferences/tabs/mcp/section';
import { SettingsSelect } from '../../src/components/settings-select';
import { Runner } from '../../src/features/preferences/tabs/local-services';
import { migrateCoreMcpConfig, readAuditLog } from '../../src/features/preferences/tabs/mcp/policy';
import {
  isStableInstall,
  runAirpostMcp,
  airpostMcpBinary,
} from '../../src/features/preferences/tabs/mcp/airpost-mcp';

// Every file here lives in a temporary profile; airpost-mcp install always gets --config-file
// pointing into it, so no real MCP client config is read or written.

let dir: string;
let coreMcp: any;
let systemTray: boolean;

function readJSON(file: string) {
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}

function mcpJSON() {
  return readJSON(path.join(dir, 'mcp.json'));
}

// Unmounted after each spec: a pane left mounted re-renders on the next spec's store changes,
// after that spec's createPortal spy is gone.
let mounted: TestRenderer.ReactTestRenderer[] = [];

async function render(props = {}) {
  let renderer: TestRenderer.ReactTestRenderer;
  await act(async () => {
    renderer = TestRenderer.create(<McpSection {...props} />);
  });
  mounted.push(renderer);
  return renderer;
}

// The DOM element with the id: kit components pass `id` down to the element they render.
function byId(renderer: TestRenderer.ReactTestRenderer, id: string) {
  return renderer.root.find((node) => typeof node.type === 'string' && node.props.id === id);
}

// Picks an option through SettingsSelect's onChange, as choosing it in the open list would.
function pick(renderer: TestRenderer.ReactTestRenderer, id: string, value: string) {
  return renderer.root
    .find((node) => node.type === SettingsSelect && node.props.id === id)
    .props.onChange(value);
}

function switchLabelled(renderer: TestRenderer.ReactTestRenderer, label: string) {
  return renderer.root.find((node) => node.type === SettingsSwitch && node.props.label === label);
}

async function flip(renderer: TestRenderer.ReactTestRenderer, label: string) {
  const sw = switchLabelled(renderer, label);
  await act(async () => {
    sw.props.onChange(!sw.props.checked);
  });
}

function section(renderer: TestRenderer.ReactTestRenderer): McpSection {
  return renderer.root.findByType(McpSection).instance;
}

/** Copy target/release/airpost-mcp to apps/desktop/airpost-mcp (or set AIRPOST_MCP_BIN) to run these. */
function noBinary() {
  if (airpostMcpBinary()) return false;
  console.warn('Skipping: airpost-mcp is not in apps/desktop and AIRPOST_MCP_BIN is not set');
  return true;
}

function textOf(renderer: TestRenderer.ReactTestRenderer) {
  return JSON.stringify(renderer.toJSON());
}

function dimmed(renderer: TestRenderer.ReactTestRenderer) {
  return renderer.root
    .findAll((n) => typeof n.type === 'string' && / off$/.test(n.props.className || ''))
    .map((n) => n.props.className);
}

/** A fake airpost that keeps the mcp-http unit's state, as `airpost mcp-http` and `airpost status` would. */
function fakeAirpost(calls: string[][]) {
  const unit = { installed: false, running: false, port: null as number | null, fail: '' };
  const run: Runner = async (args) => {
    const rest = args.slice(3);
    calls.push(rest);
    if (rest[0] === 'status') {
      const url = unit.running ? `http://127.0.0.1:${unit.port}/mcp` : null;
      const service = { name: 'mcp-http', pid: null, unit: null, bin: null, url, ...unit };
      return { ok: false, stdout: JSON.stringify({ services: [service] }), stderr: '' };
    }
    if (unit.fail) return { ok: false, stdout: '', stderr: unit.fail };
    if (rest[1] === 'install') {
      Object.assign(unit, { installed: true, running: true, port: Number(rest[3]) });
    }
    if (rest[1] === 'uninstall') Object.assign(unit, { installed: false, running: false });
    if (rest[1] === 'start') unit.running = true;
    return { ok: true, stdout: '{}', stderr: '' };
  };
  return { run, unit };
}

describe('Preferences > MCP', function () {
  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'airpost-mcp-pane-'));
    coreMcp = undefined;
    systemTray = true;
    spyOn(AppEnv, 'getConfigDirPath').andReturn(dir);
    const realGet = AppEnv.config.get.bind(AppEnv.config);
    spyOn(AppEnv.config, 'get').andCallFake((key: string) => {
      if (key === 'core.mcp') return coreMcp;
      if (key === 'core.workspace.systemTray') return systemTray;
      return realGet(key);
    });
    spyOn(AppEnv.config, 'set').andCallFake((key: string, value: any) => {
      if (key === 'core.workspace.systemTray') systemTray = value;
    });
    spyOn(AppEnv.config, 'unset').andCallFake((key: string) => {
      if (key === 'core.mcp') coreMcp = undefined;
    });
    spyOn(AccountStore, 'accounts').andReturn([
      { id: 'a1', label: 'Work', emailAddress: 'work@example.test' },
      { id: 'a2', label: 'Home', emailAddress: 'home@example.test' },
    ] as any);
    // Radix Select renders its closed list into a DocumentFragment through
    // ReactDOM.createPortal, which react-test-renderer can't host. The specs never open it.
    spyOn(ReactDOM, 'createPortal').andReturn(null);
    spyOn(CategoryStore, 'categories').andReturn([
      { id: 'f-inbox', displayName: 'Inbox' },
      { id: 'f-private', displayName: 'Private' },
    ] as any);
  });

  afterEach(() => {
    act(() => mounted.forEach((r) => r.unmount()));
    mounted = [];
    fs.rmSync(dir, { recursive: true, force: true });
  });

  describe('migrating core.mcp', () => {
    it('moves the old keys into mcp.json once, with HTTP off', () => {
      coreMcp = {
        enabled: true,
        port: 3111,
        token: 'old-token',
        accessLevel: 'read-write',
        enabledAccounts: {
          a1: { enabled: true, excludedFolderIds: null },
          a2: { enabled: null },
        },
      };
      expect(migrateCoreMcpConfig(dir)).toBe('migrated');
      expect(mcpJSON()).toEqual({
        accessLevel: 'read-write',
        enabledAccounts: {
          a1: { enabled: true, excludedFolderIds: [] },
          a2: { enabled: false, excludedFolderIds: [] },
        },
        http: { enabled: false, port: 3111 },
      });
      expect(AppEnv.config.unset).toHaveBeenCalledWith('core.mcp');
      expect(fs.readFileSync(path.join(dir, 'mcp.json'), 'utf8')).not.toContain('old-token');
      expect(migrateCoreMcpConfig(dir)).toBe('none');
    });

    it('keeps an existing mcp.json and drops the old keys', () => {
      fs.writeFileSync(path.join(dir, 'mcp.json'), JSON.stringify({ accessLevel: 'read-only' }));
      coreMcp = { accessLevel: 'read-write-send' };
      expect(migrateCoreMcpConfig(dir)).toBe('dropped');
      expect(mcpJSON()).toEqual({ accessLevel: 'read-only' });
      expect(coreMcp).toBe(undefined);
    });

    it('leaves a shape airpost-mcp refuses where it is', () => {
      spyOn(console, 'warn');
      coreMcp = { enabledAccounts: { a1: { enabled: 'yes' } } };
      expect(migrateCoreMcpConfig(dir)).toBe('none');
      expect(fs.existsSync(path.join(dir, 'mcp.json'))).toBe(false);
      expect(AppEnv.config.unset).not.toHaveBeenCalled();
    });
  });

  it('writes the access level, accounts, folders and HTTP settings to mcp.json', async () => {
    fs.writeFileSync(path.join(dir, 'mcp.json'), JSON.stringify({ futureKey: { x: 1 } }));
    const renderer = await render();

    await act(async () => {
      pick(renderer, 'mcp-access-level', 'read-write');
    });
    expect(mcpJSON().accessLevel).toBe('read-write');

    // Turning one account off lists the others as enabled, since an empty map means all.
    await flip(renderer, 'Home');
    expect(mcpJSON().enabledAccounts).toEqual({
      a1: { enabled: true, excludedFolderIds: [] },
      a2: { enabled: false, excludedFolderIds: [] },
    });

    await act(async () => {
      section(renderer)._onToggleFolder('a1', 'f-private');
    });
    expect(mcpJSON().enabledAccounts.a1).toEqual({
      enabled: true,
      excludedFolderIds: ['f-private'],
    });

    await flip(renderer, 'Allow apps over HTTP');
    await act(async () => {
      byId(renderer, 'mcp-http-port').props.onChange({ target: { value: '3000' } });
    });
    await act(async () => {
      byId(renderer, 'mcp-http-port').props.onBlur();
    });
    expect(mcpJSON().http).toEqual({ enabled: true, port: 3000 });

    // An invalid port is dropped and the saved one shown again.
    await act(async () => {
      byId(renderer, 'mcp-http-port').props.onChange({ target: { value: '70000' } });
    });
    await act(async () => {
      byId(renderer, 'mcp-http-port').props.onBlur();
    });
    expect(mcpJSON().http.port).toBe(3000);
    expect(byId(renderer, 'mcp-http-port').props.value).toBe('3000');

    expect(mcpJSON().futureKey).toEqual({ x: 1 });
    expect(mcpJSON().enabled).toBe(undefined);
    if (process.platform !== 'win32') {
      expect(fs.statSync(path.join(dir, 'mcp.json')).mode & 0o777).toBe(0o600);
    }
  });

  it('turns every app off and on, keeping Airpost running in the background when on', async () => {
    fs.writeFileSync(
      path.join(dir, 'mcp.json'),
      JSON.stringify({ accessLevel: 'read-write', enabledAccounts: { a1: { enabled: true } } })
    );
    systemTray = false;
    const renderer = await render();
    const master = 'Allow apps over stdio';
    expect(switchLabelled(renderer, master).props.checked).toBe(true);
    expect(dimmed(renderer)).toEqual([]);

    await flip(renderer, master);
    expect(mcpJSON().enabled).toBe(false);
    expect(mcpJSON().accessLevel).toBe('read-write');
    expect(mcpJSON().enabledAccounts.a1.enabled).toBe(true);
    expect(switchLabelled(renderer, master).props.checked).toBe(false);
    expect(dimmed(renderer)).toEqual(['agents-clients off', 'agents-body off']);
    expect(systemTray).toBe(false);

    await flip(renderer, master);
    expect(mcpJSON().enabled).toBe(true);
    expect(systemTray).toBe(true);
    expect(textOf(renderer)).toContain('Change in General');
  });

  it('turns HTTP on and off apart from stdio, through airpost mcp-http', async () => {
    fs.writeFileSync(path.join(dir, 'mcp.json'), JSON.stringify({ enabled: false }));
    const calls: string[][] = [];
    const { run, unit } = fakeAirpost(calls);
    const renderer = await render({ run });
    expect(calls).toEqual([['status']]);
    expect(dimmed(renderer)).toEqual(['agents-clients off', 'agents-body off']);
    expect(renderer.root.findAll((n) => n.props.id === 'mcp-http-port').length).toBe(0);

    await flip(renderer, 'Allow apps over HTTP');
    expect(mcpJSON().enabled).toBe(false);
    expect(mcpJSON().http).toEqual({ enabled: true, port: 2587 });
    expect(calls).toContain(['mcp-http', 'install', '--port', '2587']);
    // Stdio stays off; what apps can do still applies to HTTP, so it isn't dimmed.
    expect(dimmed(renderer)).toEqual(['agents-clients off']);
    expect(textOf(renderer)).toContain('Serving http://127.0.0.1:2587/mcp.');
    expect(textOf(renderer)).toContain(path.join(dir, 'mcp-token'));

    await act(async () => {
      byId(renderer, 'mcp-http-port').props.onChange({ target: { value: '3100' } });
    });
    await act(async () => {
      byId(renderer, 'mcp-http-port').props.onBlur();
    });
    expect(calls).toContain(['mcp-http', 'install', '--port', '3100']);
    expect(textOf(renderer)).toContain('Serving http://127.0.0.1:3100/mcp.');

    // Stopped from elsewhere: the pane offers Start, and shows airpost's error.
    unit.running = false;
    unit.fail = 'launchctl kickstart failed';
    await act(async () => {
      await section(renderer)._refreshHttp();
    });
    expect(textOf(renderer)).toContain('Installed, but not running.');
    await act(async () => {
      await renderer.root
        .find((n) => n.type === Button && n.props.children === 'Start')
        .props.onClick();
    });
    expect(calls).toContain(['mcp-http', 'start']);
    expect(textOf(renderer)).toContain('launchctl kickstart failed');

    unit.fail = '';
    await flip(renderer, 'Allow apps over HTTP');
    expect(mcpJSON().http.enabled).toBe(false);
    expect(calls).toContain(['mcp-http', 'uninstall']);
    expect(dimmed(renderer)).toEqual(['agents-clients off', 'agents-body off']);
  });

  it('keeps nested keys it does not know and edits made while the pane is open', async () => {
    fs.writeFileSync(
      path.join(dir, 'mcp.json'),
      JSON.stringify({
        enabledAccounts: { a1: { enabled: true, note: 'keep' } },
        http: { enabled: false, port: 2600, host: '127.0.0.1' },
      })
    );
    const renderer = await render();
    // Another writer (airpost, an editor) changes the file after the pane read it.
    const changed = mcpJSON();
    changed.accessLevel = 'read-write';
    changed.enabledAccounts.a2 = { enabled: true };
    fs.writeFileSync(path.join(dir, 'mcp.json'), JSON.stringify(changed));

    await flip(renderer, 'Allow apps over HTTP');
    const saved = mcpJSON();
    expect(saved.http).toEqual({ enabled: true, port: 2600, host: '127.0.0.1' });
    expect(saved.accessLevel).toBe('read-write');
    expect(saved.enabledAccounts).toEqual({
      a1: { enabled: true, excludedFolderIds: [], note: 'keep' },
      a2: { enabled: true, excludedFolderIds: [] },
    });
  });

  it('shows a core.mcp airpost-mcp refuses as an error and writes no wider policy', async () => {
    spyOn(console, 'warn');
    coreMcp = { enabledAccounts: { a1: { enabled: 'yes' } } };
    const renderer = await render();
    expect(textOf(renderer)).toContain('config.json is not valid');
    expect(renderer.root.findAll((n) => n.props.id === 'mcp-access-level').length).toBe(0);
    await act(async () => {
      section(renderer)._onToggleHttp();
    });
    expect(fs.existsSync(path.join(dir, 'mcp.json'))).toBe(false);
  });

  it('only checks for the stale bridge entry from an installed copy', async () => {
    const inApps = () => Promise.resolve(true);
    const elsewhere = () => Promise.resolve(false);
    expect(
      await isStableInstall({ devMode: false, platform: 'darwin', inApplicationsFolder: inApps })
    ).toBe(true);
    expect(
      await isStableInstall({ devMode: false, platform: 'darwin', inApplicationsFolder: elsewhere })
    ).toBe(false);
    expect(
      await isStableInstall({ devMode: true, platform: 'darwin', inApplicationsFolder: inApps })
    ).toBe(false);
    expect(
      await isStableInstall({ devMode: false, platform: 'linux', inApplicationsFolder: elsewhere })
    ).toBe(true);
    expect(
      await isStableInstall({ devMode: true, platform: 'win32', inApplicationsFolder: elsewhere })
    ).toBe(false);
  });

  it('shows a policy file that does not parse and never overwrites it', async () => {
    fs.writeFileSync(path.join(dir, 'mcp.json'), '{"accessLevel": ');
    const renderer = await render();
    expect(textOf(renderer)).toContain('is not valid');
    expect(renderer.root.findAll((n) => n.props.id === 'mcp-access-level').length).toBe(0);
    await act(async () => {
      section(renderer)._onToggleHttp();
    });
    expect(fs.readFileSync(path.join(dir, 'mcp.json'), 'utf8')).toBe('{"accessLevel": ');
  });

  it('shows the audit log newest first and skips damaged lines', async () => {
    const line = (toolName: string, timestamp: number, outcome = 'ok') =>
      JSON.stringify({ timestamp, client: 'claude-ai 0.1', toolName, outcome, params: '{}' });
    fs.writeFileSync(
      path.join(dir, 'mcp-audit.log'),
      [line('list_threads', 1000), '{broken', line('send_draft', 2000, 'denied'), ''].join('\n')
    );
    expect(readAuditLog(dir).map((e) => e.toolName)).toEqual(['send_draft', 'list_threads']);
    const renderer = await render();
    const text = textOf(renderer);
    expect(text.indexOf('send_draft')).toBeLessThan(text.indexOf('list_threads'));
    expect(text).toContain('denied');
  });

  it('refuses to run airpost-mcp install without --config-file in specs', async () => {
    let error = null;
    try {
      await runAirpostMcp(['install', '--client', 'claude-desktop', '--config-dir', dir]);
    } catch (err) {
      error = err;
    }
    expect(error && error.message).toContain('--config-file');
  });

  describe('with the bundled airpost-mcp', () => {
    let claudeDesktop: string;
    let claudeCode: string;
    let codex: string;

    beforeEach(() => {
      claudeDesktop = path.join(dir, 'claude_desktop_config.json');
      claudeCode = path.join(dir, 'claude-code.json');
      codex = path.join(dir, 'codex.toml');
    });

    async function renderWithFiles() {
      return render({
        configFiles: {
          'claude-desktop': claudeDesktop,
          'claude-code': claudeCode,
          codex,
        },
      });
    }

    it('adds and removes the airpost entry in temp client configs', async () => {
      if (noBinary()) return;
      fs.writeFileSync(
        claudeDesktop,
        JSON.stringify({ mcpServers: { other: { command: '/usr/local/bin/other-mcp' } } })
      );
      const renderer = await renderWithFiles();
      await act(async () => {
        await section(renderer)._onRunClient('claude-desktop', 'install');
      });
      const servers = readJSON(claudeDesktop).mcpServers;
      expect(servers.airpost.args).toEqual(['--config-dir', dir]);
      expect(path.basename(servers.airpost.command)).toBe('airpost-mcp');
      expect(servers.other).toEqual({ command: '/usr/local/bin/other-mcp' });
      expect(section(renderer).state.results['claude-desktop'].ok).toBe(true);

      await act(async () => {
        await section(renderer)._onRunClient('codex', 'install');
      });
      expect(fs.readFileSync(codex, 'utf8')).toContain('[mcp_servers.airpost]');
      await act(async () => {
        await section(renderer)._onRunClient('claude-code', 'install');
      });
      expect(readJSON(claudeCode).mcpServers.airpost.type).toBe('stdio');

      await act(async () => {
        await section(renderer)._onRunClient('claude-desktop', 'uninstall');
      });
      expect(readJSON(claudeDesktop).mcpServers.airpost).toBe(undefined);
    });

    it("replaces the old plugin's bridge entry, and offers Replace for a foreign one", async () => {
      if (noBinary()) return;
      const bridge = {
        command: '/Applications/Airpost.app/Contents/MacOS/Airpost',
        args: ['/x/app.asar.unpacked/mcp-stdio-bridge.js', '--config-dir', dir],
        env: { ELECTRON_RUN_AS_NODE: '1' },
      };
      fs.writeFileSync(claudeDesktop, JSON.stringify({ mcpServers: { airpost: bridge } }));
      const renderer = await renderWithFiles();
      await act(async () => {
        await section(renderer)._onRunClient('claude-desktop', 'install');
      });
      expect(path.basename(readJSON(claudeDesktop).mcpServers.airpost.command)).toBe('airpost-mcp');

      const foreign = { command: '/usr/local/bin/other-server' };
      fs.writeFileSync(claudeDesktop, JSON.stringify({ mcpServers: { airpost: foreign } }));
      await act(async () => {
        await section(renderer)._onRunClient('claude-desktop', 'install');
      });
      expect(readJSON(claudeDesktop).mcpServers.airpost).toEqual(foreign);
      expect(section(renderer).state.results['claude-desktop'].canForce).toBe(true);
      expect(textOf(renderer)).toContain('Replace It');
    });

    it('shows the engines airpost-mcp finds', async () => {
      if (noBinary()) return;
      const renderer = await renderWithFiles();
      await act(async () => {
        await section(renderer)._refreshStatus();
      });
      expect(section(renderer).state.status.engines).toEqual([]);
      expect(textOf(renderer)).toContain("Apps can't reach your mail right now");
    });
  });
});
