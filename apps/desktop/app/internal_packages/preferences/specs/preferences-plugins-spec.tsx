import fs from 'fs';
import os from 'os';
import path from 'path';
import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';

import { PluginsSection } from '../lib/tabs/plugins/preferences-plugins';
import { runTern, ternBinary } from '../lib/tabs/plugins/tern-cli';

// A temporary profile and resource directory: the bundled `tern` is a link to the built CLI,
// the bundled engine a stub, and every install gets --unit-dir in the temp dir, so no unit is
// written to LaunchAgents or systemd and nothing is loaded.

const SERVICE = 'sample-svc';

let root: string;
let profile: string;
let units: string;
let realTern: string | null;

function builtTern() {
  const fromEnv = process.env.TERN_BIN;
  if (fromEnv) return fs.existsSync(fromEnv) ? fromEnv : null;
  const resourcePath = AppEnv.getLoadSettings().resourcePath;
  const debug = path.resolve(resourcePath, '..', '..', '..', 'target', 'debug', 'tern');
  return fs.existsSync(debug) ? debug : null;
}

function writeExecutable(file: string, body: string) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, body);
  fs.chmodSync(file, 0o755);
}

function textOf(renderer: TestRenderer.ReactTestRenderer) {
  return JSON.stringify(renderer.toJSON());
}

function byId(renderer: TestRenderer.ReactTestRenderer, id: string) {
  return renderer.root.find((node) => node.props.id === id);
}

async function waitFor(check: () => boolean, what: string) {
  for (let i = 0; i < 200; i++) {
    if (check()) return;
    await act(() => new Promise((resolve) => setTimeout(resolve, 25)));
  }
  throw new Error(`timed out waiting for ${what}`);
}

const plugin = {
  name: SERVICE,
  displayName: 'Sample service',
  version: '0.1.0',
  directory: '',
  active: true,
  hasService: true,
};

describe('Preferences > Plugins', function () {
  beforeEach(() => {
    realTern = builtTern();
    root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'tern-plugins-pane-')));
    profile = path.join(root, 'profile');
    units = path.join(root, 'units');
    const resources = path.join(root, 'resources');
    fs.mkdirSync(resources, { recursive: true });
    writeExecutable(path.join(resources, 'mailsync-rs'), '#!/bin/sh\nexit 0\n');
    if (realTern && !process.env.TERN_BIN) fs.symlinkSync(realTern, path.join(resources, 'tern'));
    const pkg = path.join(profile, 'packages', SERVICE);
    writeExecutable(path.join(pkg, 'bin', 'svc'), '#!/bin/sh\necho v1\n');
    fs.writeFileSync(
      path.join(pkg, 'package.json'),
      JSON.stringify({
        name: SERVICE,
        engines: { mailspring: '^1.27.0' },
        tern: { service: { bin: 'bin/svc', args: ['--quiet'] } },
      })
    );
    plugin.directory = pkg;
    spyOn(AppEnv, 'getConfigDirPath').andReturn(profile);
    const settings = AppEnv.getLoadSettings();
    spyOn(AppEnv, 'getLoadSettings').andReturn({ ...settings, resourcePath: resources });
  });

  afterEach(() => {
    fs.rmSync(root, { recursive: true, force: true });
  });

  function skipWithoutTern() {
    if (realTern) return false;
    console.warn('Skipping: build target/debug/tern (cargo build -p tern) or set TERN_BIN');
    return true;
  }

  it('finds the bundled tern and refuses unit writes without --unit-dir in specs', async () => {
    if (skipWithoutTern()) return;
    expect(ternBinary()).toBeTruthy();
    let error = null;
    try {
      await runTern(['service', 'install', SERVICE]);
    } catch (err) {
      error = err;
    }
    expect(error && error.message).toContain('--unit-dir');
  });

  it('shows the exact binary, hash and units before installing, then installs and removes', async () => {
    if (skipWithoutTern()) return;
    let renderer: TestRenderer.ReactTestRenderer;
    await act(async () => {
      renderer = TestRenderer.create(<PluginsSection plugins={[plugin]} unitDir={units} />);
    });
    await waitFor(() => textOf(renderer).includes('not installed'), 'the service row');

    await act(async () => byId(renderer, `plugin-review-${SERVICE}`).props.onClick());
    await waitFor(() => textOf(renderer).includes('plugin-plan'), 'the plan');
    const plan = (renderer.root.findByType(PluginsSection).instance as PluginsSection).state.plan;
    const bin = path.join(plugin.directory, 'bin', 'svc');
    expect(plan.bin).toBe(bin);
    expect(plan.sha256).toMatch(/^[0-9a-f]{64}$/);
    expect(plan.engine.installed).toBe(false);
    expect(plan.engine.bin).toBe(path.join(root, 'resources', 'mailsync-rs'));
    expect(plan.engine.text).toContain(path.join(profile, 'engine-dev.key'));
    expect(plan.load).toBe(false);
    const shown = textOf(renderer);
    expect(shown).toContain(plan.sha256);
    expect(shown).toContain(JSON.stringify(plan.unit.text).slice(1, 60));
    expect(fs.existsSync(units)).toBe(false);

    await act(async () => byId(renderer, `plugin-install-${SERVICE}`).props.onClick());
    await waitFor(() => textOf(renderer).includes('Service installed'), 'the install');
    const copy = path.join(profile, 'services', SERVICE, 'svc');
    expect(fs.readFileSync(copy, 'utf8')).toContain('echo v1');
    expect(fs.readdirSync(units).length).toBe(2);
    const records = JSON.parse(fs.readFileSync(path.join(profile, 'services.json'), 'utf8'));
    expect(records.services[SERVICE].sha256).toBe(plan.sha256);
    expect(records.services[SERVICE].source).toBe(plugin.directory);
    await waitFor(() => textOf(renderer).includes('Uninstall service'), 'the installed row');

    await act(async () => byId(renderer, `plugin-uninstall-${SERVICE}`).props.onClick());
    await waitFor(() => textOf(renderer).includes('Service removed'), 'the removal');
    expect(fs.existsSync(copy)).toBe(false);
    expect(fs.readdirSync(units).length).toBe(1);
  });

  it('refuses to install a binary that changed after it was shown', async () => {
    if (skipWithoutTern()) return;
    let renderer: TestRenderer.ReactTestRenderer;
    await act(async () => {
      renderer = TestRenderer.create(<PluginsSection plugins={[plugin]} unitDir={units} />);
    });
    await waitFor(() => textOf(renderer).includes('not installed'), 'the service row');
    await act(async () => byId(renderer, `plugin-review-${SERVICE}`).props.onClick());
    await waitFor(() => textOf(renderer).includes('plugin-plan'), 'the plan');

    writeExecutable(path.join(plugin.directory, 'bin', 'svc'), '#!/bin/sh\necho swapped\n');
    await act(async () => byId(renderer, `plugin-install-${SERVICE}`).props.onClick());
    await waitFor(() => textOf(renderer).includes('changed'), 'the refusal');
    expect(fs.existsSync(path.join(profile, 'services', SERVICE))).toBe(false);
    expect(fs.existsSync(path.join(profile, 'services.json'))).toBe(false);
  });

  it('refuses to install when the unit would differ from the one shown', async () => {
    if (skipWithoutTern()) return;
    let renderer: TestRenderer.ReactTestRenderer;
    await act(async () => {
      renderer = TestRenderer.create(<PluginsSection plugins={[plugin]} unitDir={units} />);
    });
    await waitFor(() => textOf(renderer).includes('not installed'), 'the service row');
    await act(async () => byId(renderer, `plugin-review-${SERVICE}`).props.onClick());
    await waitFor(() => textOf(renderer).includes('plugin-plan'), 'the plan');
    const plan = (renderer.root.findByType(PluginsSection).instance as PluginsSection).state.plan;
    expect(plan.planSha256).toMatch(/^[0-9a-f]{64}$/);

    // Same binary, other args: only the plan digest catches it.
    const manifest = path.join(plugin.directory, 'package.json');
    const json = JSON.parse(fs.readFileSync(manifest, 'utf8'));
    json.tern.service.args = ['--loud'];
    fs.writeFileSync(manifest, JSON.stringify(json));
    await act(async () => byId(renderer, `plugin-install-${SERVICE}`).props.onClick());
    await waitFor(() => textOf(renderer).includes('changed since it was shown'), 'the refusal');
    expect(fs.existsSync(units)).toBe(false);
    expect(fs.existsSync(path.join(profile, 'services.json'))).toBe(false);
  });
});
