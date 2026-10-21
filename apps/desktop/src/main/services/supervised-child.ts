// SPDX-License-Identifier: GPL-3.0-only
import { EventEmitter } from 'events';
import { ChildProcess, spawn } from 'child_process';
import type { ServiceStatus } from '../../platform/services';

export interface ChildSpec {
  command: string;
  args: string[];
  env?: NodeJS.ProcessEnv;
}

export interface SupervisedChildOptions {
  name: string;
  /** A stderr line that says the child is serving; its URL, when it names one. */
  ready: (line: string) => { url: string | null } | null;
  initialDelayMs?: number;
  maxDelayMs?: number;
  /** A child that ran this long before it exited is restarted after the initial delay again. */
  stableMs?: number;
  stopTimeoutMs?: number;
}

/**
 * A server process main runs while the app runs (tern-mcp http, tern-gateway serve). It is
 * started for a spec, restarted with a doubling delay when it exits, restarted at once when
 * the spec changes and stopped with a null spec. Status changes are emitted as 'change'.
 */
export class SupervisedChild extends EventEmitter {
  private spec: ChildSpec | null = null;
  private child: ChildProcess | null = null;
  private retry: NodeJS.Timeout | null = null;
  private delayMs: number;
  private startedAt = 0;
  private lastLine = '';
  private current: ServiceStatus = { state: 'stopped', url: null, error: null, pid: null };
  private queue: Promise<void> = Promise.resolve();
  private readonly opts: Required<SupervisedChildOptions>;

  constructor(opts: SupervisedChildOptions) {
    super();
    this.opts = {
      initialDelayMs: 1000,
      maxDelayMs: 60000,
      stableMs: 30000,
      stopTimeoutMs: 3000,
      ...opts,
    };
    this.delayMs = this.opts.initialDelayMs;
  }

  status(): ServiceStatus {
    return this.current;
  }

  /** Runs `spec`, or stops with null. The same spec again leaves a running child alone. */
  setSpec(spec: ChildSpec | null): Promise<void> {
    this.queue = this.queue.then(async () => {
      if (JSON.stringify(spec) === JSON.stringify(this.spec)) return;
      this.spec = spec;
      this.delayMs = this.opts.initialDelayMs;
      this.clearRetry();
      await this.kill();
      if (spec) this.start(spec);
      else this.update({ state: 'stopped', url: null, error: null, pid: null });
    });
    return this.queue;
  }

  stop() {
    return this.setSpec(null);
  }

  /** Sends SIGTERM at once and forgets the child, for the app's quit. */
  terminate() {
    this.spec = null;
    this.clearRetry();
    const child = this.child;
    this.child = null;
    if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
  }

  private update(next: ServiceStatus) {
    this.current = next;
    this.emit('change', next);
  }

  private clearRetry() {
    if (this.retry) clearTimeout(this.retry);
    this.retry = null;
  }

  private start(spec: ChildSpec) {
    this.lastLine = '';
    this.startedAt = Date.now();
    let child: ChildProcess;
    try {
      child = spawn(spec.command, spec.args, {
        env: { ...process.env, ...spec.env },
        stdio: ['ignore', 'ignore', 'pipe'],
      });
    } catch (err) {
      this.exited(null, (err as Error).message);
      return;
    }
    this.child = child;
    this.update({ state: 'starting', url: null, error: null, pid: child.pid || null });
    let buffered = '';
    child.stderr?.on('data', (chunk) => {
      buffered += chunk.toString();
      const lines = buffered.split('\n');
      buffered = lines.pop() || '';
      for (const line of lines) this.onLine(child, line);
    });
    child.on('error', (err) => {
      if (this.child === child) this.exited(child, err.message);
    });
    child.on('exit', (code, signal) => {
      if (this.child !== child) return;
      this.exited(child, this.lastLine || `exited with ${signal || `code ${code}`}`);
    });
  }

  private onLine(child: ChildProcess, raw: string) {
    // tracing colours the line when stderr is a terminal; it isn't here, but be safe.
    // eslint-disable-next-line no-control-regex
    const line = raw.replace(/\x1b\[[0-9;]*m/g, '').trim();
    if (!line || this.child !== child) return;
    this.lastLine = line;
    const ready = this.opts.ready(line);
    if (ready && this.current.state !== 'running') {
      this.update({ state: 'running', url: ready.url, error: null, pid: child.pid || null });
    }
  }

  private exited(child: ChildProcess | null, error: string) {
    if (child && this.child !== child) return;
    this.child = null;
    const spec = this.spec;
    if (!spec) return;
    console.warn(`[${this.opts.name}] ${error}`);
    if (Date.now() - this.startedAt >= this.opts.stableMs) this.delayMs = this.opts.initialDelayMs;
    const delay = this.delayMs;
    this.delayMs = Math.min(this.delayMs * 2, this.opts.maxDelayMs);
    this.update({ state: 'error', url: null, error, pid: null });
    this.retry = setTimeout(() => {
      this.retry = null;
      if (this.spec === spec && !this.child) this.start(spec);
    }, delay);
  }

  private kill(): Promise<void> {
    const child = this.child;
    this.child = null;
    if (!child || child.exitCode !== null || child.signalCode !== null) return Promise.resolve();
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        child.kill('SIGKILL');
        resolve();
      }, this.opts.stopTimeoutMs);
      child.once('exit', () => {
        clearTimeout(timer);
        resolve();
      });
      child.kill('SIGTERM');
    });
  }
}
