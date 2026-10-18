// Builds the app in gateway mode and starts e2e/stack.py (fake mail servers, a `serve` engine
// and tern-gateway on a throwaway profile under /tmp), unless E2E_STACK names the stack.json of
// one already running. The returned teardown stops what this started.
import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const WEB = path.resolve(import.meta.dirname, '..');

export default async function globalSetup(): Promise<(() => Promise<void>) | void> {
  if (process.env.E2E_STACK) return;
  const work = fs.mkdtempSync(path.join('/tmp', 'tern-web-e2e-'));
  const webRoot = path.join(work, 'web');
  execFileSync(
    'npx',
    ['vite', 'build', '--outDir', webRoot, '--emptyOutDir', '--logLevel', 'warn'],
    {
      cwd: WEB,
      env: { ...process.env, VITE_TERN_ENGINE: 'gateway' },
      stdio: 'inherit',
    },
  );
  const log = fs.openSync(path.join(work, 'stack.log'), 'a');
  const stack = spawn(
    'python3',
    [path.join(WEB, 'e2e', 'stack.py'), '--web-root', webRoot, '--work', work],
    { stdio: ['ignore', log, log] },
  );
  const info = path.join(work, 'stack.json');
  const deadline = Date.now() + 240_000;
  while (!fs.existsSync(info)) {
    if (stack.exitCode !== null) throw new Error(`stack.py exited; see ${work}/stack.log`);
    if (Date.now() > deadline) {
      stack.kill('SIGTERM');
      throw new Error(`stack.py didn't start in time; see ${work}/stack.log`);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  process.env.E2E_STACK = info;
  console.log(`e2e stack in ${work} (${os.hostname()})`);
  return async () => {
    stack.kill('SIGTERM');
    await new Promise((r) => (stack.exitCode !== null ? r(null) : stack.once('exit', r)));
  };
}
