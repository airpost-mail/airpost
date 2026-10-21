import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';
import { shell } from 'electron';
import { Button } from '../../src/components/kit/button';
import { SettingsSwitch } from '../../src/components/settings';
import { ServicesRows } from '../../src/features/preferences/tabs/services-rows';
import { LocalService, Runner } from '../../src/features/preferences/tabs/local-services';
import * as localServices from '../../src/features/preferences/tabs/local-services';
import * as Actions from '../../src/data/actions';
import { engineBinaryPath } from '../../src/engine/process';

// tern is a fake that keeps the units' state, as `tern <service> install|uninstall|start` and
// `tern --json status` would; nothing is installed or run.

let mounted: TestRenderer.ReactTestRenderer[] = [];

async function render(run: Runner) {
  let renderer: TestRenderer.ReactTestRenderer;
  await act(async () => {
    renderer = TestRenderer.create(<ServicesRows run={run} />);
  });
  mounted.push(renderer);
  return renderer;
}

const textOf = (r: TestRenderer.ReactTestRenderer) => JSON.stringify(r.toJSON());

/** The first button with the text: the engine's Install comes before the tern command's. */
function button(r: TestRenderer.ReactTestRenderer, text: string) {
  return r.root.findAll((n) => n.type === Button && n.props.children === text)[0];
}

async function click(r: TestRenderer.ReactTestRenderer, text: string) {
  await act(async () => {
    await button(r, text).props.onClick();
  });
}

function fakeTern(calls: string[][], { fail = '', services = true } = {}) {
  const units: { [name: string]: LocalService } = {};
  for (const name of ['engine', 'gateway', 'mcp-http'] as const) {
    units[name] = {
      name,
      installed: false,
      running: false,
      pid: null,
      url: null,
      port: null,
      unit: null,
      bin: null,
    };
  }
  const run: Runner = async (args) => {
    const rest = args.slice(3);
    calls.push(rest);
    if (rest[0] === 'status') {
      const body = services ? { services: Object.values(units) } : { engines: [] };
      return { ok: true, stdout: JSON.stringify(body), stderr: '' };
    }
    if (fail) return { ok: false, stdout: '', stderr: fail };
    const [name, action] = rest;
    if (name === 'gateway' && action === 'pair') {
      return {
        ok: true,
        stdout: JSON.stringify({ code: 'ABCD-1234', url: 'http://127.0.0.1:5180/pair#ABCD-1234' }),
        stderr: '',
      };
    }
    const unit = units[name];
    if (action === 'install') {
      unit.installed = true;
      // The engine unit is written without being loaded.
      unit.running = name !== 'engine';
      if (name === 'gateway') unit.url = 'http://127.0.0.1:5180';
    }
    if (action === 'uninstall') Object.assign(unit, { installed: false, running: false });
    return { ok: true, stdout: '{}', stderr: '' };
  };
  return { run, units };
}

describe('Preferences > General > Services on this computer', function () {
  let missing: string[];

  beforeEach(() => {
    missing = [];
    spyOn(localServices, 'webAccessMissing').andCallFake(() => missing);
  });

  afterEach(() => {
    act(() => mounted.forEach((r) => r.unmount()));
    mounted = [];
  });

  it('installs the engine unit without starting it, and removes it', async () => {
    const calls: string[][] = [];
    const { run } = fakeTern(calls);
    const r = await render(run);
    expect(textOf(r)).toContain('Keep Tern running in the background');
    expect(textOf(r)).toContain('tern-engine');

    await click(r, 'Install');
    const install = calls.find((c) => c[0] === 'engine' && c[1] === 'install') as string[];
    const { resourcePath } = AppEnv.getLoadSettings();
    expect(install.slice(0, 5)).toEqual([
      'engine',
      'install',
      '--bin',
      engineBinaryPath(resourcePath),
      '--no-start',
    ]);
    expect(textOf(r)).toContain('It starts when you quit Tern and when you log in.');

    await click(r, 'Uninstall');
    expect(calls).toContain(['engine', 'uninstall']);
    expect(button(r, 'Install')).toBeTruthy();
  });

  it('shows why tern refused', async () => {
    const calls: string[][] = [];
    const { run } = fakeTern(calls, { fail: 'these services run on the engine unit: x' });
    const r = await render(run);
    await click(r, 'Install');
    expect(textOf(r)).toContain('these services run on the engine unit: x');
  });

  it('turns web access on with the bundled web client, and pairs a browser', async () => {
    const calls: string[][] = [];
    const { run } = fakeTern(calls);
    spyOn(shell, 'openExternal');
    const r = await render(run);
    const toggle = () =>
      r.root.find(
        (n) => n.type === SettingsSwitch && n.props.label === 'Web access from this computer'
      );
    expect(toggle().props.checked).toBe(false);
    await act(async () => {
      await toggle().props.onChange(true);
    });
    const install = calls.find((c) => c[0] === 'gateway' && c[1] === 'install') as string[];
    expect(install.slice(0, 5)).toEqual([
      'gateway',
      'install',
      '--bind',
      '127.0.0.1:5180',
      '--web-root',
    ]);
    expect(toggle().props.checked).toBe(true);
    expect(textOf(r)).toContain('Open http://127.0.0.1:5180 in a browser.');

    await click(r, 'Pair a Browser');
    expect(calls).toContain(['gateway', 'pair']);
    expect(textOf(r)).toContain('ABCD-1234');
    await click(r, 'Open Link');
    expect(shell.openExternal).toHaveBeenCalledWith('http://127.0.0.1:5180/pair#ABCD-1234');

    await act(async () => {
      await toggle().props.onChange(false);
    });
    expect(calls).toContain(['gateway', 'uninstall']);
  });

  it('says when this copy has no web client', async () => {
    missing = ['tern-gateway', 'web'];
    const r = await render(fakeTern([]).run);
    expect(textOf(r)).toContain('Not included');
    expect(
      r.root.findAll(
        (n) => n.type === SettingsSwitch && n.props.label === 'Web access from this computer'
      ).length
    ).toBe(0);
  });

  it('says when tern is too old to list services', async () => {
    const r = await render(fakeTern([], { services: false }).run);
    expect(textOf(r)).toContain('This tern doesn’t list services yet.');
    const web = r.root.find(
      (n) => n.type === SettingsSwitch && n.props.label === 'Web access from this computer'
    );
    expect(web.props.disabled).toBe(true);
  });

  it('reflects MCP over HTTP and links to the MCP tab', async () => {
    const { run, units } = fakeTern([]);
    Object.assign(units['mcp-http'], {
      installed: true,
      running: true,
      url: 'http://127.0.0.1:2587/mcp',
    });
    spyOn(Actions, 'switchPreferencesTab');
    const r = await render(run);
    expect(textOf(r)).toContain('Serving http://127.0.0.1:2587/mcp.');
    r.root.find((n) => n.type === 'a').props.onClick();
    expect(Actions.switchPreferencesTab).toHaveBeenCalledWith('Agents');
  });
});
