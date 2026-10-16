import fs from 'fs';
import os from 'os';
import path from 'path';
import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';
import { AccountStore, CategoryStore } from 'mailspring-exports';

import { McpSection } from '../lib/tabs/mcp/mcp-section';
import { migrateCoreMcpConfig, readAuditLog } from '../lib/tabs/mcp/mcp-policy';
import { isStableInstall, runTernMcp, ternMcpBinary } from '../lib/tabs/mcp/tern-mcp';

// Every file here lives in a temporary profile; tern-mcp install always gets --config-file
// pointing into it, so no real MCP client config is read or written.

let dir: string;
let coreMcp: any;

function readJSON(file: string) {
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}

function mcpJSON() {
  return readJSON(path.join(dir, 'mcp.json'));
}

async function render(props = {}) {
  let renderer: TestRenderer.ReactTestRenderer;
  await act(async () => {
    renderer = TestRenderer.create(<McpSection {...props} />);
  });
  return renderer;
}

function byId(renderer: TestRenderer.ReactTestRenderer, id: string) {
  return renderer.root.find((node) => node.props.id === id);
}

function section(renderer: TestRenderer.ReactTestRenderer): McpSection {
  return renderer.root.findByType(McpSection).instance;
}

/** Copy target/release/tern-mcp to app/tern-mcp (or set TERN_MCP_BIN) to run these. */
function noBinary() {
  if (ternMcpBinary()) return false;
  console.warn('Skipping: tern-mcp is not in app/ and TERN_MCP_BIN is not set');
  return true;
}

function textOf(renderer: TestRenderer.ReactTestRenderer) {
  return JSON.stringify(renderer.toJSON());
}

describe('Preferences > Integrations > AI assistants', function () {
  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tern-mcp-pane-'));
    coreMcp = undefined;
    spyOn(AppEnv, 'getConfigDirPath').andReturn(dir);
    const realGet = AppEnv.config.get.bind(AppEnv.config);
    spyOn(AppEnv.config, 'get').andCallFake((key: string) =>
      key === 'core.mcp' ? coreMcp : realGet(key)
    );
    spyOn(AppEnv.config, 'unset').andCallFake((key: string) => {
      if (key === 'core.mcp') coreMcp = undefined;
    });
    spyOn(AccountStore, 'accounts').andReturn([
      { id: 'a1', label: 'Work', emailAddress: 'work@example.test' },
      { id: 'a2', label: 'Home', emailAddress: 'home@example.test' },
    ] as any);
    spyOn(CategoryStore, 'categories').andReturn([
      { id: 'f-inbox', displayName: 'Inbox' },
      { id: 'f-private', displayName: 'Private' },
    ] as any);
  });

  afterEach(() => {
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

    it('leaves a shape tern-mcp refuses where it is', () => {
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
      byId(renderer, 'mcp-access-level').props.onChange({ target: { value: 'read-write' } });
    });
    expect(mcpJSON().accessLevel).toBe('read-write');

    // Turning one account off lists the others as enabled, since an empty map means all.
    await act(async () => {
      byId(renderer, 'mcp-account-a2').props.onChange();
    });
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

    await act(async () => {
      byId(renderer, 'mcp-http-enabled').props.onChange();
    });
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
    if (process.platform !== 'win32') {
      expect(fs.statSync(path.join(dir, 'mcp.json')).mode & 0o777).toBe(0o600);
    }
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
    // Another writer (tern, an editor) changes the file after the pane read it.
    const changed = mcpJSON();
    changed.accessLevel = 'read-write';
    changed.enabledAccounts.a2 = { enabled: true };
    fs.writeFileSync(path.join(dir, 'mcp.json'), JSON.stringify(changed));

    await act(async () => {
      byId(renderer, 'mcp-http-enabled').props.onChange();
    });
    const saved = mcpJSON();
    expect(saved.http).toEqual({ enabled: true, port: 2600, host: '127.0.0.1' });
    expect(saved.accessLevel).toBe('read-write');
    expect(saved.enabledAccounts).toEqual({
      a1: { enabled: true, excludedFolderIds: [], note: 'keep' },
      a2: { enabled: true, excludedFolderIds: [] },
    });
  });

  it('shows a core.mcp tern-mcp refuses as an error and writes no wider policy', async () => {
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

  it('only checks for the stale bridge entry from an installed copy', () => {
    const inApps = () => true;
    const elsewhere = () => false;
    expect(
      isStableInstall({ devMode: false, platform: 'darwin', inApplicationsFolder: inApps })
    ).toBe(true);
    expect(
      isStableInstall({ devMode: false, platform: 'darwin', inApplicationsFolder: elsewhere })
    ).toBe(false);
    expect(
      isStableInstall({ devMode: true, platform: 'darwin', inApplicationsFolder: inApps })
    ).toBe(false);
    expect(
      isStableInstall({ devMode: false, platform: 'linux', inApplicationsFolder: elsewhere })
    ).toBe(true);
    expect(
      isStableInstall({ devMode: true, platform: 'win32', inApplicationsFolder: elsewhere })
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

  it('refuses to run tern-mcp install without --config-file in specs', async () => {
    let error = null;
    try {
      await runTernMcp(['install', '--client', 'claude-desktop', '--config-dir', dir]);
    } catch (err) {
      error = err;
    }
    expect(error && error.message).toContain('--config-file');
  });

  describe('with the bundled tern-mcp', () => {
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

    it('adds and removes the tern entry in temp client configs', async () => {
      if (noBinary()) return;
      fs.writeFileSync(
        claudeDesktop,
        JSON.stringify({ mcpServers: { mailspring: { command: '/Applications/Mailspring.app' } } })
      );
      const renderer = await renderWithFiles();
      await act(async () => {
        await section(renderer)._onRunClient('claude-desktop', 'install');
      });
      const servers = readJSON(claudeDesktop).mcpServers;
      expect(servers.tern.args).toEqual(['--config-dir', dir]);
      expect(path.basename(servers.tern.command)).toBe('tern-mcp');
      expect(servers.mailspring).toEqual({ command: '/Applications/Mailspring.app' });
      expect(section(renderer).state.results['claude-desktop'].ok).toBe(true);

      await act(async () => {
        await section(renderer)._onRunClient('codex', 'install');
      });
      expect(fs.readFileSync(codex, 'utf8')).toContain('[mcp_servers.tern]');
      await act(async () => {
        await section(renderer)._onRunClient('claude-code', 'install');
      });
      expect(readJSON(claudeCode).mcpServers.tern.type).toBe('stdio');

      await act(async () => {
        await section(renderer)._onRunClient('claude-desktop', 'uninstall');
      });
      expect(readJSON(claudeDesktop).mcpServers.tern).toBe(undefined);
    });

    it("replaces the old plugin's bridge entry, and offers Replace for a foreign one", async () => {
      if (noBinary()) return;
      const bridge = {
        command: '/Applications/Tern.app/Contents/MacOS/Tern',
        args: ['/x/app.asar.unpacked/mcp-stdio-bridge.js', '--config-dir', dir],
        env: { ELECTRON_RUN_AS_NODE: '1' },
      };
      fs.writeFileSync(claudeDesktop, JSON.stringify({ mcpServers: { tern: bridge } }));
      const renderer = await renderWithFiles();
      await act(async () => {
        await section(renderer)._onRunClient('claude-desktop', 'install');
      });
      expect(path.basename(readJSON(claudeDesktop).mcpServers.tern.command)).toBe('tern-mcp');

      const foreign = { command: '/usr/local/bin/other-server' };
      fs.writeFileSync(claudeDesktop, JSON.stringify({ mcpServers: { tern: foreign } }));
      await act(async () => {
        await section(renderer)._onRunClient('claude-desktop', 'install');
      });
      expect(readJSON(claudeDesktop).mcpServers.tern).toEqual(foreign);
      expect(section(renderer).state.results['claude-desktop'].canForce).toBe(true);
      expect(textOf(renderer)).toContain('Replace It');
    });

    it('shows the engines tern-mcp finds', async () => {
      if (noBinary()) return;
      const renderer = await renderWithFiles();
      await act(async () => {
        await section(renderer)._refreshStatus();
      });
      expect(section(renderer).state.status.engines).toEqual([]);
      expect(textOf(renderer)).toContain('No engine running');
    });
  });
});
