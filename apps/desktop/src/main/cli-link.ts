// SPDX-License-Identifier: GPL-3.0-only
import fs from 'fs';
import os from 'os';
import path from 'path';
import { execFile } from 'child_process';
import type { CliLinkResult, CliLinkStatus } from '../platform/cli-link';

/*
Installs the `tern` command as a symlink to the bundled binary, so updating the app updates the
command. macOS links /usr/local/bin/tern, which is on the default PATH and usually needs an
administrator password (asked through osascript); when that fails for a reason other than the
user cancelling, it links ~/.local/bin/tern instead. Linux links ~/.local/bin/tern. Only links
are replaced or removed: a real file at the path is left alone.
*/

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

function isOurs(binary: string, loc: string) {
  const real = realpath(binary);
  return !!real && !!lstat(loc)?.isSymbolicLink() && realpath(loc) === real;
}

export function linkStatus(binary: string | null, locations: string[]): CliLinkStatus {
  if (!binary || locations.length === 0) return { state: 'unsupported', binary, path: null };
  const installed = locations.find((loc) => isOurs(binary, loc));
  if (installed) return { state: 'installed', binary, path: installed };
  const preferred = locations[0];
  const stat = lstat(preferred);
  if (!stat) return { state: 'missing', binary, path: preferred };
  if (stat.isSymbolicLink()) {
    const otherTarget = fs.readlinkSync(preferred);
    return { state: 'other', binary, path: preferred, otherTarget, replaceable: true };
  }
  return { state: 'other', binary, path: preferred, otherTarget: preferred, replaceable: false };
}

function isPermissionError(err: unknown) {
  const code = (err as NodeJS.ErrnoException).code;
  return code === 'EACCES' || code === 'EPERM';
}

/** Links `loc` to `binary`, replacing a link but never a file. */
function linkAt(binary: string, loc: string) {
  fs.mkdirSync(path.dirname(loc), { recursive: true });
  const stat = lstat(loc);
  if (stat && !stat.isSymbolicLink()) throw new Error(`${loc} is not a link`);
  if (stat) fs.unlinkSync(loc);
  fs.symlinkSync(binary, loc);
}

function linkCommand(binary: string, loc: string) {
  const l = shellQuote(loc);
  return `if [ -e ${l} ] && [ ! -L ${l} ]; then exit 1; fi; /bin/mkdir -p ${shellQuote(path.dirname(loc))} && /bin/ln -sfn ${shellQuote(binary)} ${l}`;
}

export async function installLink(
  binary: string | null,
  locations: string[],
  { replace = false, runAdmin = null }: { replace?: boolean; runAdmin?: AdminRunner | null } = {}
): Promise<CliLinkResult> {
  const before = linkStatus(binary, locations);
  if (before.state === 'installed') return { ok: true, status: before };
  if (before.state === 'unsupported') return { ok: false, error: 'unsupported', status: before };
  if (before.state === 'other' && !(replace && before.replaceable)) {
    return { ok: false, error: 'taken', status: before };
  }
  const bin = binary as string;
  const [preferred, ...fallbacks] = locations;
  const done = () => ({ ok: true, status: linkStatus(binary, locations) });
  try {
    linkAt(bin, preferred);
    return done();
  } catch (err) {
    if (!isPermissionError(err)) {
      return { ok: false, error: (err as Error).message, status: linkStatus(binary, locations) };
    }
  }
  let error = 'permission denied';
  if (runAdmin) {
    const result = await runAdmin(linkCommand(bin, preferred));
    if (result.ok) return done();
    if (result.cancelled) {
      return { ok: false, cancelled: true, status: linkStatus(binary, locations) };
    }
    error = result.error;
  }
  for (const loc of fallbacks) {
    const stat = lstat(loc);
    if (stat && !(stat.isSymbolicLink() && (replace || isOurs(bin, loc)))) continue;
    try {
      linkAt(bin, loc);
      return done();
    } catch (err) {
      error = (err as Error).message;
    }
  }
  return { ok: false, error, status: linkStatus(binary, locations) };
}

export async function uninstallLink(
  binary: string | null,
  locations: string[],
  { runAdmin = null }: { runAdmin?: AdminRunner | null } = {}
): Promise<CliLinkResult> {
  const status = linkStatus(binary, locations);
  if (status.state !== 'installed') return { ok: true, status };
  const loc = status.path as string;
  try {
    fs.unlinkSync(loc);
  } catch (err) {
    if (!isPermissionError(err) || !runAdmin) {
      return { ok: false, error: (err as Error).message, status };
    }
    const l = shellQuote(loc);
    const result = await runAdmin(`if [ -L ${l} ]; then /bin/rm ${l}; fi`);
    if (!result.ok) {
      return { ok: false, cancelled: result.cancelled, error: result.error, status };
    }
  }
  return { ok: true, status: linkStatus(binary, locations) };
}
