// SPDX-License-Identifier: GPL-3.0-only
import fs from 'fs';
import os from 'os';
import path from 'path';
import {
  AdminRunner,
  installLink,
  linkLocations,
  linkStatus,
  uninstallLink,
} from '../../src/main/cli-link';

describe('the tern command link', function () {
  let root: string;
  let binary: string;
  let system: string;
  let user: string;
  let locations: string[];

  beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), 'tern-cli-link-'));
    binary = path.join(root, 'Tern.app', 'tern');
    fs.mkdirSync(path.dirname(binary));
    fs.writeFileSync(binary, '#!/bin/sh\n');
    system = path.join(root, 'usr-local-bin', 'tern');
    user = path.join(root, 'home', '.local', 'bin', 'tern');
    locations = [system, user];
  });

  afterEach(() => {
    fs.chmodSync(root, 0o755);
    const sysDir = path.dirname(system);
    if (fs.existsSync(sysDir)) fs.chmodSync(sysDir, 0o755);
    fs.rmSync(root, { recursive: true, force: true });
  });

  /** Makes the preferred folder unwritable, as /usr/local/bin is without an admin password. */
  function lockSystemDir() {
    fs.mkdirSync(path.dirname(system), { recursive: true });
    fs.chmodSync(path.dirname(system), 0o555);
  }

  it('links /usr/local/bin first on macOS, ~/.local/bin only on Linux, nothing on Windows', () => {
    expect(linkLocations('darwin', '/Users/a')).toEqual([
      '/usr/local/bin/tern',
      '/Users/a/.local/bin/tern',
    ]);
    expect(linkLocations('linux', '/home/a')).toEqual(['/home/a/.local/bin/tern']);
    expect(linkLocations('win32', 'C:\\Users\\a')).toEqual([]);
  });

  it('reports a missing link at the preferred place, and no binary as unsupported', () => {
    expect(linkStatus(binary, locations)).toEqual({ state: 'missing', binary, path: system });
    expect(linkStatus(null, locations).state).toBe('unsupported');
    expect(linkStatus(binary, []).state).toBe('unsupported');
  });

  it('installs a link to the binary and removes it again', async () => {
    const installed = await installLink(binary, locations);
    expect(installed.ok).toBe(true);
    expect(installed.status).toEqual({ state: 'installed', binary, path: system });
    expect(fs.readlinkSync(system)).toBe(binary);

    const removed = await uninstallLink(binary, locations);
    expect(removed.ok).toBe(true);
    expect(fs.existsSync(system)).toBe(false);
    expect(removed.status.state).toBe('missing');
  });

  it('replaces a link to another copy only when asked', async () => {
    const old = path.join(root, 'Old.app', 'tern');
    fs.mkdirSync(path.dirname(old));
    fs.writeFileSync(old, '');
    fs.mkdirSync(path.dirname(system), { recursive: true });
    fs.symlinkSync(old, system);

    const status = linkStatus(binary, locations);
    expect(status).toEqual({
      state: 'other',
      binary,
      path: system,
      otherTarget: old,
      replaceable: true,
    });
    const refused = await installLink(binary, locations);
    expect(refused.ok).toBe(false);
    expect(refused.error).toBe('taken');
    expect(fs.readlinkSync(system)).toBe(old);

    const replaced = await installLink(binary, locations, { replace: true });
    expect(replaced.ok).toBe(true);
    expect(fs.readlinkSync(system)).toBe(binary);
  });

  it('never replaces or removes a real file', async () => {
    fs.mkdirSync(path.dirname(system), { recursive: true });
    fs.writeFileSync(system, 'someone else');
    expect(linkStatus(binary, locations).replaceable).toBe(false);
    const result = await installLink(binary, locations, { replace: true });
    expect(result.ok).toBe(false);
    expect(fs.readFileSync(system, 'utf8')).toBe('someone else');
    await uninstallLink(binary, locations);
    expect(fs.readFileSync(system, 'utf8')).toBe('someone else');
  });

  it('asks for an administrator when the folder is not writable', async () => {
    lockSystemDir();
    const commands: string[] = [];
    const runAdmin: AdminRunner = async (command) => {
      commands.push(command);
      return { ok: false, cancelled: true, error: 'User canceled. (-128)' };
    };
    const result = await installLink(binary, locations, { runAdmin });
    expect(commands.length).toBe(1);
    expect(commands[0]).toContain(`/bin/ln -sfn '${binary}' '${system}'`);
    // Cancelling the prompt doesn't fall back to ~/.local/bin.
    expect(result.ok).toBe(false);
    expect(result.cancelled).toBe(true);
    expect(fs.existsSync(user)).toBe(false);
  });

  it('falls back to ~/.local/bin when the administrator prompt fails', async () => {
    lockSystemDir();
    const runAdmin: AdminRunner = async () => ({ ok: false, cancelled: false, error: 'denied' });
    const result = await installLink(binary, locations, { runAdmin });
    expect(result.ok).toBe(true);
    expect(result.status).toEqual({ state: 'installed', binary, path: user });
    expect(fs.readlinkSync(user)).toBe(binary);
  });
});
