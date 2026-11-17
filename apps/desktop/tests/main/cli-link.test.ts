// SPDX-License-Identifier: GPL-3.0-only
import fs from 'fs';
import os from 'os';
import path from 'path';
import { execFileSync } from 'child_process';
import {
  AdminRunner,
  cliDefaultProfile,
  installLink,
  linkLocations,
  linkStatus,
  profileToPin,
  uninstallLink,
  wrapperScript,
} from '../../src/main/cli-link';

describe('the airpost command link', function () {
  let root: string;
  let binary: string;
  let system: string;
  let user: string;
  let locations: string[];

  beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), 'airpost-cli-link-'));
    binary = path.join(root, 'Airpost.app', 'airpost');
    fs.mkdirSync(path.dirname(binary));
    fs.writeFileSync(binary, '#!/bin/sh\n');
    system = path.join(root, 'usr-local-bin', 'airpost');
    user = path.join(root, 'home', '.local', 'bin', 'airpost');
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
      '/usr/local/bin/airpost',
      '/Users/a/.local/bin/airpost',
    ]);
    expect(linkLocations('linux', '/home/a')).toEqual(['/home/a/.local/bin/airpost']);
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
    const old = path.join(root, 'Old.app', 'airpost');
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
      otherProfile: null,
      sameBinary: false,
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

describe('the airpost command for a profile other than the default one', function () {
  let root: string;
  let binary: string;
  let profile: string;
  let system: string;
  let user: string;
  let locations: string[];

  beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), 'airpost-cli-wrapper-'));
    binary = path.join(root, 'Airpost.app', 'airpost');
    fs.mkdirSync(path.dirname(binary));
    fs.writeFileSync(binary, '#!/bin/sh\necho "$AIRPOST_CONFIG_DIR|$*"\n', { mode: 0o755 });
    profile = path.join(root, "airpost trial's profile");
    fs.mkdirSync(profile);
    system = path.join(root, 'usr-local-bin', 'airpost');
    user = path.join(root, 'home', '.local', 'bin', 'airpost');
    locations = [system, user];
  });

  afterEach(() => {
    const sysDir = path.dirname(system);
    if (fs.existsSync(sysDir)) fs.chmodSync(sysDir, 0o755);
    fs.rmSync(root, { recursive: true, force: true });
  });

  function lockSystemDir() {
    fs.mkdirSync(path.dirname(system), { recursive: true });
    fs.chmodSync(path.dirname(system), 0o555);
  }

  const run = (file: string, env: NodeJS.ProcessEnv = {}) => {
    const clean = { ...process.env };
    delete clean.AIRPOST_CONFIG_DIR;
    return execFileSync(file, ['status', '--json'], { env: { ...clean, ...env } })
      .toString()
      .trim();
  };

  it('works out the profile airpost uses on its own, as airpost-client does', () => {
    expect(cliDefaultProfile('darwin', { HOME: '/Users/a' })).toBe(
      '/Users/a/Library/Application Support/Airpost'
    );
    expect(cliDefaultProfile('linux', { HOME: '/home/a' })).toBe('/home/a/.config/Airpost');
    expect(cliDefaultProfile('linux', { HOME: '/home/a', XDG_CONFIG_HOME: '/x' })).toBe(
      '/x/Airpost'
    );
    expect(cliDefaultProfile('linux', { HOME: '/home/a', XDG_CONFIG_HOME: '' })).toBe(
      '/home/a/.config/Airpost'
    );
    expect(cliDefaultProfile('darwin', {})).toBe(null);

    expect(profileToPin(path.join(root, 'Airpost'), path.join(root, 'Airpost'))).toBe(null);
    expect(profileToPin(profile, path.join(root, 'Airpost'))).toBe(profile);
    expect(profileToPin('relative', null)).toBe(path.resolve('relative'));
  });

  it('installs a wrapper that pins the profile unless AIRPOST_CONFIG_DIR is set', async () => {
    const result = await installLink(binary, locations, { profile });
    expect(result.ok).toBe(true);
    expect(result.status).toEqual({ state: 'installed', binary, path: system, profile });
    expect(fs.lstatSync(system).isSymbolicLink()).toBe(false);
    expect(fs.readFileSync(system, 'utf8')).toBe(wrapperScript(binary, profile));
    expect(run(system)).toBe(`${profile}|status --json`);
    expect(run(system, { AIRPOST_CONFIG_DIR: '/elsewhere' })).toBe('/elsewhere|status --json');

    const removed = await uninstallLink(binary, locations, { profile });
    expect(removed.ok).toBe(true);
    expect(fs.existsSync(system)).toBe(false);
  });

  it('offers to replace a link to this copy with the wrapper, leaving the binary alone', async () => {
    fs.mkdirSync(path.dirname(system), { recursive: true });
    fs.symlinkSync(binary, system);
    const before = fs.readFileSync(binary, 'utf8');

    const status = linkStatus(binary, locations, { profile });
    expect(status.state).toBe('other');
    expect(status.replaceable).toBe(true);
    expect(status.sameBinary).toBe(true);
    expect(status.otherProfile).toBe(null);
    expect((await installLink(binary, locations, { profile })).error).toBe('taken');

    const replaced = await installLink(binary, locations, { profile, replace: true });
    expect(replaced.ok).toBe(true);
    expect(replaced.status.state).toBe('installed');
    expect(fs.lstatSync(system).isSymbolicLink()).toBe(false);
    expect(fs.readFileSync(binary, 'utf8')).toBe(before);
    expect(run(system)).toBe(`${profile}|status --json`);
  });

  it('treats a wrapper for another profile or binary as replaceable', async () => {
    await installLink(binary, locations, { profile });
    const other = path.join(root, 'other-profile');
    const status = linkStatus(binary, locations, { profile: other });
    expect(status.state).toBe('other');
    expect(status.replaceable).toBe(true);
    expect(status.otherProfile).toBe(profile);
    expect(status.sameBinary).toBe(true);
    // Uninstalling for another profile leaves this one's wrapper.
    await uninstallLink(binary, locations, { profile: other });
    expect(fs.existsSync(system)).toBe(true);

    const otherBinary = path.join(root, 'Old.app', 'airpost');
    fs.mkdirSync(path.dirname(otherBinary));
    fs.writeFileSync(otherBinary, '');
    expect(linkStatus(otherBinary, locations, { profile }).sameBinary).toBe(false);

    // On the default profile a plain link replaces the wrapper.
    const plain = linkStatus(binary, locations);
    expect(plain.state).toBe('other');
    expect(plain.replaceable).toBe(true);
    expect((await installLink(binary, locations, { replace: true })).ok).toBe(true);
    expect(fs.readlinkSync(system)).toBe(binary);
  });

  it('never replaces or removes a script that is not Airpost’s', async () => {
    fs.mkdirSync(path.dirname(system), { recursive: true });
    fs.writeFileSync(system, `#!/bin/sh\nexec /opt/other/airpost "$@"\n`);
    expect(linkStatus(binary, locations, { profile }).replaceable).toBe(false);
    expect((await installLink(binary, locations, { profile, replace: true })).ok).toBe(false);
    await uninstallLink(binary, locations, { profile });
    expect(fs.readFileSync(system, 'utf8')).toContain('/opt/other/airpost');
  });

  it('asks an administrator to move the wrapper into place', async () => {
    lockSystemDir();
    const commands: string[] = [];
    const runAdmin: AdminRunner = async (command) => {
      commands.push(command);
      return { ok: false, cancelled: true, error: 'User canceled. (-128)' };
    };
    const result = await installLink(binary, locations, { profile, runAdmin });
    expect(result.cancelled).toBe(true);
    expect(commands.length).toBe(1);
    expect(commands[0]).toContain(`/usr/bin/grep -qF '# airpost-command-wrapper'`);
    expect(commands[0]).toContain(`/bin/mv -f '${system}.airpost-new' '${system}'`);
    expect(commands[0]).not.toContain('/bin/ln');
    expect(fs.existsSync(user)).toBe(false);
  });

  it('runs the administrator command it builds', async () => {
    // The same command, run without root on a folder that is writable after all.
    fs.mkdirSync(path.dirname(system), { recursive: true });
    fs.symlinkSync(binary, system);
    fs.chmodSync(path.dirname(system), 0o555);
    const runAdmin: AdminRunner = async (command) => {
      fs.chmodSync(path.dirname(system), 0o755);
      execFileSync('/bin/sh', ['-c', command]);
      return { ok: true, cancelled: false, error: '' };
    };
    const result = await installLink(binary, locations, { profile, replace: true, runAdmin });
    expect(result.ok).toBe(true);
    expect(result.status.path).toBe(system);
    expect(run(system)).toBe(`${profile}|status --json`);
  });

  it('falls back to a wrapper in ~/.local/bin', async () => {
    lockSystemDir();
    const result = await installLink(binary, locations, { profile });
    expect(result.ok).toBe(true);
    expect(result.status.path).toBe(user);
    expect(run(user)).toBe(`${profile}|status --json`);
  });
});
