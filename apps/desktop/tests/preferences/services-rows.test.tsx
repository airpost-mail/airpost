import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';
import { ipcRenderer, shell } from 'electron';
import { Button } from '../../src/components/kit/button';
import { SettingsSwitch } from '../../src/components/settings';
import {
  EngineUnitRow,
  McpHttpRow,
  WebAccessRows,
} from '../../src/features/preferences/tabs/services-rows';
import {
  EngineUnitStatus,
  GatewayStatus,
  McpHttpStatus,
  ServicesChannel,
} from '../../src/platform/services';
import * as Actions from '../../src/data/actions';

// main/ipc/services.ts is replaced by a spy on ipcRenderer.invoke; nothing is installed or run.

let answers: { [channel: string]: (...args: any[]) => unknown };
let calls: [string, ...unknown[]][];
let mounted: TestRenderer.ReactTestRenderer[] = [];
let webAccess: boolean;

async function render(element: React.ReactElement) {
  let renderer: TestRenderer.ReactTestRenderer;
  await act(async () => {
    renderer = TestRenderer.create(element);
  });
  mounted.push(renderer);
  return renderer;
}

const textOf = (r: TestRenderer.ReactTestRenderer) => JSON.stringify(r.toJSON());

function button(r: TestRenderer.ReactTestRenderer, text: string) {
  return r.root.find((n) => n.type === Button && n.props.children === text);
}

async function click(r: TestRenderer.ReactTestRenderer, text: string) {
  await act(async () => {
    await button(r, text).props.onClick();
  });
}

const unit = (over: Partial<EngineUnitStatus> = {}): EngineUnitStatus => ({
  available: true,
  installed: false,
  running: false,
  joined: false,
  unit: null,
  ...over,
});

const gateway = (over: Partial<GatewayStatus> = {}): GatewayStatus => ({
  service: 'gateway',
  state: 'stopped',
  url: null,
  error: null,
  pid: null,
  enabled: false,
  available: true,
  missing: [],
  ...over,
});

describe('Preferences > General > Services on this computer', function () {
  beforeEach(() => {
    answers = {};
    calls = [];
    webAccess = false;
    spyOn(ipcRenderer, 'invoke').andCallFake((channel: string, ...args: unknown[]) => {
      calls.push([channel, ...args]);
      const answer = answers[channel];
      return Promise.resolve(answer ? answer(...args) : null);
    });
    const realGet = AppEnv.config.get.bind(AppEnv.config);
    spyOn(AppEnv.config, 'get').andCallFake((key: string) =>
      key === 'core.services.webAccess' ? webAccess : realGet(key)
    );
    spyOn(AppEnv.config, 'set').andCallFake((key: string, value: any) => {
      if (key === 'core.services.webAccess') webAccess = value;
    });
  });

  afterEach(() => {
    act(() => mounted.forEach((r) => r.unmount()));
    mounted = [];
  });

  it('installs and uninstalls the engine unit, and says when it runs', async () => {
    let installed = false;
    answers[ServicesChannel.EngineUnitStatus] = () => unit({ installed });
    answers[ServicesChannel.EngineUnitSet] = (install: boolean) => {
      installed = install;
      return { ok: true, status: unit({ installed, running: false }) };
    };
    const r = await render(<EngineUnitRow />);
    expect(textOf(r)).toContain('Keep Tern running in the background');
    expect(textOf(r)).toContain('tern-engine');

    await click(r, 'Install');
    expect(calls).toContain([ServicesChannel.EngineUnitSet, true]);
    expect(textOf(r)).toContain('It starts when you quit Tern and when you log in.');
    expect(textOf(r)).toContain('Installed');

    await click(r, 'Uninstall');
    expect(calls).toContain([ServicesChannel.EngineUnitSet, false]);
    expect(button(r, 'Install')).toBeTruthy();
  });

  it('shows why tern refused to change the unit', async () => {
    answers[ServicesChannel.EngineUnitStatus] = () => unit({ installed: true, running: true });
    answers[ServicesChannel.EngineUnitSet] = () => ({
      ok: false,
      error: 'these services run on the engine unit; uninstall them first: x',
      status: unit({ installed: true, running: true }),
    });
    const r = await render(<EngineUnitRow />);
    expect(textOf(r)).toContain('It is running.');
    await click(r, 'Uninstall');
    expect(textOf(r)).toContain('uninstall them first: x');
  });

  it('turns web access on, shows its address and pairs a browser', async () => {
    answers[ServicesChannel.GatewayStatus] = () => gateway();
    answers[ServicesChannel.GatewayPair] = () => ({
      ok: true,
      code: 'ABCD-1234',
      url: 'http://127.0.0.1:5180/pair#ABCD-1234',
      expiresAt: 1,
    });
    spyOn(shell, 'openExternal');
    const r = await render(<WebAccessRows />);
    const toggle = r.root.findByType(SettingsSwitch);
    expect(toggle.props.checked).toBe(false);
    await act(async () => toggle.props.onChange(true));
    expect(webAccess).toBe(true);

    // Main broadcasts the gateway's state once it serves.
    await act(async () => {
      r.root.findByType(WebAccessRows).instance.setState({
        status: gateway({ enabled: true, state: 'running', url: 'http://127.0.0.1:5180' }),
      });
    });
    expect(textOf(r)).toContain('Open http://127.0.0.1:5180 in a browser.');
    await click(r, 'Pair a Browser');
    expect(textOf(r)).toContain('ABCD-1234');
    await click(r, 'Open Link');
    expect(shell.openExternal).toHaveBeenCalledWith('http://127.0.0.1:5180/pair#ABCD-1234');
  });

  it('says when this copy has no web client', async () => {
    answers[ServicesChannel.GatewayStatus] = () =>
      gateway({ available: false, missing: ['tern-gateway', 'web'] });
    const r = await render(<WebAccessRows />);
    expect(textOf(r)).toContain('Not included');
    expect(r.root.findAllByType(SettingsSwitch).length).toBe(0);
  });

  it('reflects MCP over HTTP and links to the MCP tab', async () => {
    const http: McpHttpStatus = {
      service: 'mcp-http',
      state: 'running',
      url: 'http://127.0.0.1:2587/mcp',
      error: null,
      pid: 1,
      enabled: true,
      port: 2587,
      available: true,
      tokenFile: '/x/mcp-token',
      policyError: null,
    };
    answers[ServicesChannel.McpHttpStatus] = () => http;
    spyOn(Actions, 'switchPreferencesTab');
    const r = await render(<McpHttpRow />);
    expect(textOf(r)).toContain('Serving http://127.0.0.1:2587/mcp.');
    expect(r.root.findAllByType(SettingsSwitch).length).toBe(0);
    r.root.find((n) => n.type === 'a').props.onClick();
    expect(Actions.switchPreferencesTab).toHaveBeenCalledWith('Agents');
  });
});
