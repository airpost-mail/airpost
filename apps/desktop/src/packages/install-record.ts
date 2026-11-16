import crypto from 'crypto';
import fs from 'fs';
import path from 'path';

// Where each plugin installed through Airpost came from and what it was (TD29, TD30): the
// directory the user picked and a SHA-256 over the copied files, shown in Preferences >
// Plugins. Like the engine-method approvals this is a record, not a security boundary: a
// plugin runs as trusted code and could rewrite it.

export const PLUGIN_INSTALLS_KEY = 'core.plugins.installs';

export interface PluginInstallRecord {
  source: string;
  sha256: string;
  installedAt: number;
}

function walk(root: string, rel: string, out: string[]) {
  const entries = fs.readdirSync(path.join(root, rel), { withFileTypes: true });
  for (const entry of entries) {
    const child = rel ? `${rel}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      walk(root, child, out);
    } else {
      out.push(child);
    }
  }
}

/** SHA-256 over every file's relative path and content (a symlink's target, not followed),
 * in sorted order, so the same tree always hashes the same. */
export function hashPackageDir(dir: string): string {
  const files: string[] = [];
  walk(dir, '', files);
  files.sort();
  const hash = crypto.createHash('sha256');
  for (const rel of files) {
    const full = path.join(dir, rel);
    const stat = fs.lstatSync(full);
    const content = stat.isSymbolicLink()
      ? Buffer.from(`symlink:${fs.readlinkSync(full)}`)
      : fs.readFileSync(full);
    const fileHash = crypto.createHash('sha256').update(content).digest('hex');
    hash.update(`${rel}\0${fileHash}\n`);
  }
  return hash.digest('hex');
}

export function recordPluginInstall(name: string, source: string, installedDir: string) {
  const record: PluginInstallRecord = {
    source,
    sha256: hashPackageDir(installedDir),
    installedAt: Date.now(),
  };
  const all = AppEnv.config.get(PLUGIN_INSTALLS_KEY) || {};
  AppEnv.config.set(PLUGIN_INSTALLS_KEY, { ...all, [name]: record });
  return record;
}

export function pluginInstallRecord(name: string): PluginInstallRecord | null {
  const record = (AppEnv.config.get(PLUGIN_INSTALLS_KEY) || {})[name];
  return record && typeof record.sha256 === 'string' ? record : null;
}
