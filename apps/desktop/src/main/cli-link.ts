// SPDX-License-Identifier: GPL-3.0-only
import fs from 'fs';
import os from 'os';
import path from 'path';
import { execFile } from 'child_process';
import type { CliLinkResult, CliLinkStatus } from '../platform/cli-link';

/*
Installs the `tern` command, so updating the app updates the command. On the app's own
default profile (the one `tern` picks without --config-dir, `default_config_dirs` in
crates/tern-client/src/discovery.rs) it is a symlink to the bundled binary. On any other
profile (`--config-dir-path`, `--dev`) it is a small sh wrapper that sets TERN_CONFIG_DIR, so
`tern` reaches this app's engine. The wrapper's second line is a marker naming the binary and
profile, which is how status, replace and uninstall know it is Tern's.

macOS installs at /usr/local/bin/tern, which is on the default PATH and usually needs an
administrator password (asked through osascript); when that fails for a reason other than the
user cancelling, it installs at ~/.local/bin/tern instead. Linux uses ~/.local/bin/tern. Only
links and Tern's wrappers are replaced or removed: any other file at the path is left alone.
*/

/**
 * The profile `tern` uses without --config-dir: the first of `default_config_dirs` in
 * crates/tern-client/src/discovery.rs, which is Electron's appData plus `Tern`. (`tern`
 * prefers a later one only when that one has a live engine and the first doesn't.)
 */
export function cliDefaultProfile(
  platform: string = process.platform,
  env: NodeJS.ProcessEnv = process.env
): string | null {
  const home = env.HOME || null;
  let appData: string | null;
  if (platform === 'darwin') {
    appData = home && path.join(home, 'Library', 'Application Support');
  } else if (platform === 'win32') {
    appData = env.APPDATA || null;
  } else {
    appData = env.XDG_CONFIG_HOME || (home && path.join(home, '.config'));
  }
  return appData ? path.join(appData, 'Tern') : null;
}

function samePath(a: string, b: string) {
  return (realpath(a) || path.resolve(a)) === (realpath(b) || path.resolve(b));
}

/** The profile the installed command has to pin: null when `tern` finds it on its own. */
export function profileToPin(configDir: string, defaultProfile = cliDefaultProfile()) {
  return defaultProfile && samePath(configDir, defaultProfile) ? null : path.resolve(configDir);
}

/** Where the link goes, preferred first; the rest are fallbacks. */
export function linkLocations(platform: string = process.platform, home = os.homedir()) {
  const user = path.join(home, '.local', 'bin', 'tern');
  if (platform === 'darwin') return ['/usr/local/bin/tern', user];
  if (platform === 'linux') return [user];
  return [];
}

/** Runs a shell command as root; null where there's no way to ask. */
export type AdminRunner = (
  command: string
) => Promise<{ ok: boolean; cancelled: boolean; error: string }>;

const shellQuote = (s: string) => `'${s.replace(/'/g, `'\\''`)}'`;
const appleScriptString = (s: string) => `"${s.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;

/** macOS's administrator prompt, through `do shell script`. */
export function osascriptAdminRunner(prompt: string): AdminRunner {
  return (command) =>
    new Promise((resolve) => {
      const script = `do shell script ${appleScriptString(command)} with prompt ${appleScriptString(prompt)} with administrator privileges`;
      execFile('/usr/bin/osascript', ['-e', script], (err, _stdout, stderr) => {
        if (!err) return resolve({ ok: true, cancelled: false, error: '' });
        const text = String(stderr || err.message).trim();
        // -128 is "User canceled."
        resolve({ ok: false, cancelled: /\(-128\)/.test(text), error: text });
      });
    });
}

function realpath(p: string) {
  try {
    return fs.realpathSync(p);
  } catch (err) {
    return null;
  }
}

function lstat(p: string) {
  try {
    return fs.lstatSync(p);
  } catch (err) {
    return null;
  }
}

export const WRAPPER_MARKER = '# tern-command-wrapper ';

/** The wrapper installed for a profile other than the default one. */
export function wrapperScript(binary: string, profile: string) {
  return [
    '#!/bin/sh',
    WRAPPER_MARKER + JSON.stringify({ binary, profile }),
    '# Installed by Tern (Preferences > General), which replaces or removes it.',
    `if [ -z "\${TERN_CONFIG_DIR:-}" ]; then TERN_CONFIG_DIR=${shellQuote(profile)}; export TERN_CONFIG_DIR; fi`,
    `exec ${shellQuote(binary)} "$@"`,
    '',
  ].join('\n');
}

type Entry =
  | { kind: 'link'; target: string }
  | { kind: 'wrapper'; binary: string; profile: string }
  | { kind: 'file' };

function readWrapper(loc: string): { binary: string; profile: string } | null {
  let head: string;
  try {
    const fd = fs.openSync(loc, 'r');
    try {
      const buffer = Buffer.alloc(8192);
      head = buffer.toString('utf8', 0, fs.readSync(fd, buffer, 0, buffer.length, 0));
    } finally {
      fs.closeSync(fd);
    }
  } catch (err) {
    return null;
  }
  const line = head.split('\n')[1] || '';
  if (!line.startsWith(WRAPPER_MARKER)) return null;
  try {
    const { binary, profile } = JSON.parse(line.slice(WRAPPER_MARKER.length));
    return typeof binary === 'string' && typeof profile === 'string' ? { binary, profile } : null;
  } catch (err) {
    return null;
  }
}

function entryAt(loc: string): Entry | null {
  const stat = lstat(loc);
  if (!stat) return null;
  if (stat.isSymbolicLink()) return { kind: 'link', target: fs.readlinkSync(loc) };
  const wrapper = stat.isFile() ? readWrapper(loc) : null;
  return wrapper ? { kind: 'wrapper', ...wrapper } : { kind: 'file' };
}

const isTerns = (e: Entry | null) => !!e && e.kind !== 'file';

function isInstalledAt(binary: string, profile: string | null, loc: string) {
  const entry = entryAt(loc);
  if (!entry || entry.kind === 'file') return false;
  if (profile === null) return entry.kind === 'link' && samePath(loc, binary);
  return (
    entry.kind === 'wrapper' && samePath(entry.binary, binary) && samePath(entry.profile, profile)
  );
}

export interface Target {
  /** The profile to pin with a wrapper; null installs a plain link. */
  profile?: string | null;
}

export function linkStatus(
  binary: string | null,
  locations: string[],
  { profile = null }: Target = {}
): CliLinkStatus {
  if (!binary || locations.length === 0) return { state: 'unsupported', binary, path: null };
  const pinned = profile ? { profile } : {};
  const installed = locations.find((loc) => isInstalledAt(binary, profile, loc));
  if (installed) return { state: 'installed', binary, path: installed, ...pinned };
  const preferred = locations[0];
  const entry = entryAt(preferred);
  if (!entry) return { state: 'missing', binary, path: preferred, ...pinned };
  if (entry.kind === 'file') {
    return {
      state: 'other',
      binary,
      path: preferred,
      ...pinned,
      otherTarget: preferred,
      replaceable: false,
    };
  }
  return {
    state: 'other',
    binary,
    path: preferred,
    ...pinned,
    otherTarget: entry.kind === 'link' ? entry.target : entry.binary,
    otherProfile: entry.kind === 'wrapper' ? entry.profile : null,
    sameBinary: samePath(entry.kind === 'link' ? preferred : entry.binary, binary),
    replaceable: true,
  };
}

function isPermissionError(err: unknown) {
  const code = (err as NodeJS.ErrnoException).code;
  return code === 'EACCES' || code === 'EPERM';
}

/** Puts the link or wrapper at `loc`, replacing a link or a wrapper but no other file. The
 * wrapper goes in through a rename, which replaces a link rather than writing through it. */
function installAt(binary: string, profile: string | null, loc: string) {
  fs.mkdirSync(path.dirname(loc), { recursive: true });
  const entry = entryAt(loc);
  if (entry && !isTerns(entry)) throw new Error(`${loc} is not Tern's`);
  if (profile === null) {
    if (entry) fs.unlinkSync(loc);
    fs.symlinkSync(binary, loc);
    return;
  }
  const tmp = `${loc}.tern-${process.pid}.tmp`;
  try {
    fs.writeFileSync(tmp, wrapperScript(binary, profile), { mode: 0o755 });
    fs.renameSync(tmp, loc);
  } catch (err) {
    fs.rmSync(tmp, { force: true });
    throw err;
  }
}

/** sh that succeeds when `loc` is missing, a link or one of Tern's wrappers. */
const isTernsTest = (l: string) =>
  `{ [ ! -e ${l} ] || [ -L ${l} ] || /usr/bin/grep -qF ${shellQuote(WRAPPER_MARKER.trim())} ${l}; }`;

function adminInstallCommand(binary: string, loc: string, wrapperFile: string | null) {
  const l = shellQuote(loc);
  const dir = `/bin/mkdir -p ${shellQuote(path.dirname(loc))}`;
  if (!wrapperFile) {
    return `if [ -e ${l} ] && [ ! -L ${l} ]; then exit 1; fi; ${dir} && /bin/ln -sfn ${shellQuote(binary)} ${l}`;
  }
  const next = shellQuote(`${loc}.tern-new`);
  return (
    `${isTernsTest(l)} || exit 1; ${dir} && /bin/cp ${shellQuote(wrapperFile)} ${next} && ` +
    `/bin/chmod 755 ${next} && { [ ! -L ${l} ] || /bin/rm -f ${l}; } && /bin/mv -f ${next} ${l}`
  );
}

async function installAsAdmin(
  runAdmin: AdminRunner,
  binary: string,
  profile: string | null,
  loc: string
) {
  if (profile === null) return runAdmin(adminInstallCommand(binary, loc, null));
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tern-command-'));
  try {
    const file = path.join(dir, 'tern');
    fs.writeFileSync(file, wrapperScript(binary, profile), { mode: 0o644 });
    return await runAdmin(adminInstallCommand(binary, loc, file));
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

export async function installLink(
  binary: string | null,
  locations: string[],
  {
    replace = false,
    runAdmin = null,
    profile = null,
  }: { replace?: boolean; runAdmin?: AdminRunner | null } & Target = {}
): Promise<CliLinkResult> {
  const status = () => linkStatus(binary, locations, { profile });
  const before = status();
  if (before.state === 'installed') return { ok: true, status: before };
  if (before.state === 'unsupported') return { ok: false, error: 'unsupported', status: before };
  if (before.state === 'other' && !(replace && before.replaceable)) {
    return { ok: false, error: 'taken', status: before };
  }
  const bin = binary as string;
  const [preferred, ...fallbacks] = locations;
  const done = () => ({ ok: true, status: status() });
  try {
    installAt(bin, profile, preferred);
    return done();
  } catch (err) {
    if (!isPermissionError(err)) {
      return { ok: false, error: (err as Error).message, status: status() };
    }
  }
  let error = 'permission denied';
  if (runAdmin) {
    const result = await installAsAdmin(runAdmin, bin, profile, preferred);
    if (result.ok) return done();
    if (result.cancelled) return { ok: false, cancelled: true, status: status() };
    error = result.error;
  }
  for (const loc of fallbacks) {
    const entry = entryAt(loc);
    if (entry && !(isTerns(entry) && (replace || isInstalledAt(bin, profile, loc)))) continue;
    try {
      installAt(bin, profile, loc);
      return done();
    } catch (err) {
      error = (err as Error).message;
    }
  }
  return { ok: false, error, status: status() };
}

export async function uninstallLink(
  binary: string | null,
  locations: string[],
  { runAdmin = null, profile = null }: { runAdmin?: AdminRunner | null } & Target = {}
): Promise<CliLinkResult> {
  const status = linkStatus(binary, locations, { profile });
  if (status.state !== 'installed') return { ok: true, status };
  const loc = status.path as string;
  try {
    fs.unlinkSync(loc);
  } catch (err) {
    if (!isPermissionError(err) || !runAdmin) {
      return { ok: false, error: (err as Error).message, status };
    }
    const l = shellQuote(loc);
    const result = await runAdmin(
      `if [ -e ${l} ] || [ -L ${l} ]; then ${isTernsTest(l)} && /bin/rm -f ${l}; fi`
    );
    if (!result.ok) {
      return { ok: false, cancelled: result.cancelled, error: result.error, status };
    }
  }
  return { ok: true, status: linkStatus(binary, locations, { profile }) };
}
