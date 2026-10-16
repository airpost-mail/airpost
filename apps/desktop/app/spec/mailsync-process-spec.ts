import { EventEmitter } from 'events';
import fs from 'fs';
import net from 'net';
import os from 'os';
import path from 'path';
import { lastJSONResponse, MailsyncProcess } from '../src/mailsync-process';
import { engineSocketPath } from '../src/engine-protocol';

describe('lastJSONResponse', function () {
  it('reads a result that has no trailing newline', function () {
    expect(lastJSONResponse('{"error":null}')).toEqual({ error: null });
  });

  it('skips the progress lines mailsync prints before the result', function () {
    expect(lastJSONResponse('Running Setup\n{"error":null}')).toEqual({ error: null });
  });

  it('tolerates a trailing newline and blank lines', function () {
    expect(lastJSONResponse('{"error":null}\n\n')).toEqual({ error: null });
  });

  it('returns the last result when several objects were written', function () {
    expect(lastJSONResponse('{"error":"first"}\n{"error":"second"}\n')).toEqual({
      error: 'second',
    });
  });

  it('returns null when nothing on stdout is JSON', function () {
    expect(lastJSONResponse('Error: Could not load libtidy.\nInstall libtidy.\n')).toBe(null);
  });

  it('reads a result written with Windows line endings', function () {
    // mailsync's stdout is in text mode on Windows, so every newline arrives as CRLF and
    // the result line carries a trailing carriage return.
    expect(lastJSONResponse('Running Setup\r\n{"error":null}\r\n')).toEqual({ error: null });
  });

  it('returns null for empty output', function () {
    expect(lastJSONResponse('')).toBe(null);
  });

  it('ignores a truncated object at the end of the stream', function () {
    expect(lastJSONResponse('{"error":null}\n{"error":')).toEqual({ error: null });
  });
});

describe('accumulating mailsync stdout', function () {
  // mailsync's result line carries the whole IMAP/SMTP conversation in `test` mode, so it
  // routinely spans more than one 64KB chunk, and a server error or account name is often
  // non-ASCII. Decoding each chunk on its own splits any multi-byte character that lands on
  // a boundary into two replacement characters.
  const payload = '{"error":"Café serveur"}';
  const bytes = Buffer.from(payload, 'utf-8');
  const splitAt = bytes.indexOf(0xc3) + 1; // between the two bytes of 'é'
  const chunks = [bytes.subarray(0, splitAt), bytes.subarray(splitAt)];

  it('mangles a character split across chunks when each chunk is decoded alone', function () {
    const perChunk = chunks.map((c) => c.toString('utf-8')).join('');
    expect(perChunk).not.toEqual(payload);
    expect(perChunk).toContain('�');
    expect(lastJSONResponse(perChunk).error).not.toEqual('Café serveur');
  });

  it('preserves it when the bytes are joined before decoding', function () {
    const joined = Buffer.concat(chunks).toString('utf-8');
    expect(joined).toEqual(payload);
    expect(lastJSONResponse(joined)).toEqual({ error: 'Café serveur' });
  });
});

describe('interpreting a mailsync exit', function () {
  let proc: MailsyncProcess = null;
  let child: any = null;

  beforeEach(function () {
    proc = new MailsyncProcess({ configDirPath: '/tmp', resourcePath: '/tmp', verbose: false });
    child = new EventEmitter();
    child.stdout = new EventEmitter();
    child.stderr = new EventEmitter();
    spyOn(proc as any, '_spawnProcess').andCallFake(() => {
      (proc as any)._proc = child;
    });
  });

  const errorFromTest = (stdout: string, code: number) => {
    const promise = proc.test();
    child.stdout.emit('data', Buffer.from(stdout, 'utf-8'));
    child.emit('close', code, null);
    return promise.then(
      () => {
        throw new Error('expected `test` to reject');
      },
      (err) => err
    );
  };

  it('flags a failure the engine classified as this machine being offline', async function () {
    const err = await errorFromTest(
      '{"error":"ErrorConnection","error_service":"imap","error_offline":true,"log":"..."}',
      1
    );
    expect(err.isNetworkError).toBe(true);
  });

  it('does not flag a failure the server itself reported', async function () {
    const err = await errorFromTest(
      '{"error":"ErrorAuthentication","error_service":"imap","error_offline":false,"log":"..."}',
      1
    );
    expect(err.isNetworkError).toBe(false);
  });

  it('does not flag a result from an engine that predates the field', async function () {
    const err = await errorFromTest('{"error":"ErrorConnection","error_service":"imap"}', 1);
    expect(err.isNetworkError).toBe(false);
  });

  it('reports a crash as an unknown error, whatever the dump contains', async function () {
    const err = await errorFromTest('terminate called\n{"offline":true,"retryable":true', 134);
    expect(err.message).toContain('mailsync: 134');
    expect(err.isNetworkError).toBe(undefined);
  });
});

describe('migrating for the Rust engine', function () {
  it('runs import-legacy after migrate and launches anyway when it fails', async function () {
    const proc = new MailsyncProcess({
      configDirPath: '/tmp',
      resourcePath: '/tmp',
      syncEngine: 'rust',
    });
    const modes: string[] = [];
    spyOn(proc as any, '_spawnProcess').andCallFake((mode: string) => {
      modes.push(mode);
      const child: any = new EventEmitter();
      child.stdout = new EventEmitter();
      child.stderr = new EventEmitter();
      (proc as any)._proc = child;
      const ok = mode === 'migrate';
      // A microtask, not setTimeout: the spec helper fakes timers, and the listeners are
      // attached right after _spawnProcess returns.
      Promise.resolve().then(() => {
        const result = ok ? '\n{"error":null}' : '\n{"error":"no Message table"}';
        child.stdout.emit('data', Buffer.from(result, 'utf-8'));
        child.emit('close', ok ? 0 : 1, null);
      });
    });
    spyOn(console, 'error');

    await proc.migrate();
    expect(modes).toEqual(['migrate', 'import-legacy']);
    expect(console.error).toHaveBeenCalled();
  });
});

describe('the engine socket path', function () {
  it('is <configDir>/run/<accountId>.sock when it fits', function () {
    expect(engineSocketPath('/Users/me/.tern', 'a1b2')).toEqual({
      socketPath: '/Users/me/.tern/run/a1b2.sock',
      ownDir: null,
    });
  });

  it('falls back to a short name in a per-user temp dir', function () {
    const configDir = `/Users/me/${'x'.repeat(100)}`;
    const { socketPath, ownDir } = engineSocketPath(configDir, 'a1b2');
    expect(ownDir).toEqual(path.join(os.tmpdir(), `tern-${process.getuid()}`));
    expect(path.dirname(socketPath)).toEqual(ownDir);
    expect(path.basename(socketPath)).toMatch(/^[0-9a-f]{12}\.sock$/);
    expect(Buffer.byteLength(socketPath)).not.toBeGreaterThan(103);
  });
});

describe('syncing over the engine socket', function () {
  const ACCOUNT_ID = 'acc1';
  let dir: string = null;
  let socketPath: string = null;
  let server: net.Server = null;
  let conn: net.Socket = null;
  let received: any[] = [];
  let onReceived: () => void = null;
  let proc: MailsyncProcess = null;
  let child: any = null;
  let spawnArgs: string[] = null;
  let savedTransport: string = undefined;

  // Resolves once the fake engine has read `n` request lines in total.
  const requests = (n: number) =>
    new Promise<any[]>((resolve) => {
      const check = () => {
        if (received.length >= n) {
          onReceived = null;
          resolve(received.slice(0, n));
        }
      };
      onReceived = check;
      check();
    });

  const nextEvent = (emitter: EventEmitter, event: string) =>
    new Promise<any>((resolve) => emitter.once(event, resolve));

  const startFakeEngine = () =>
    new Promise<void>((resolve) => {
      server = net.createServer((socket) => {
        conn = socket;
        let buffer = '';
        socket.setEncoding('utf8');
        socket.on('data', (chunk: string) => {
          buffer += chunk;
          const lines = buffer.split('\n');
          buffer = lines.pop();
          for (const line of lines) {
            const req = JSON.parse(line);
            received.push(req);
            const result = req.method === 'calendar.sync' ? { started: true } : {};
            socket.write(`${JSON.stringify({ jsonrpc: '2.0', id: req.id, result })}\n`);
          }
          if (onReceived) onReceived();
        });
      });
      server.listen(socketPath, () => resolve());
    });

  const listening = () => {
    child.stdout.emit('data', Buffer.from(`${JSON.stringify({ listening: socketPath })}\n`));
  };

  beforeEach(async function () {
    savedTransport = process.env.TERN_ENGINE_TRANSPORT;
    delete process.env.TERN_ENGINE_TRANSPORT;
    // Short, unlike os.tmpdir() on macOS, so <configDir>/run/<id>.sock is the path used.
    dir = fs.mkdtempSync('/tmp/tern-spec-');
    fs.mkdirSync(path.join(dir, 'run'));
    socketPath = path.join(dir, 'run', `${ACCOUNT_ID}.sock`);
    received = [];
    conn = null;

    proc = new MailsyncProcess({ configDirPath: dir, resourcePath: dir, syncEngine: 'rust' });
    proc.account = { id: ACCOUNT_ID, emailAddress: 'me@example.com' } as any;
    child = new EventEmitter();
    child.stdout = new EventEmitter();
    child.stderr = new EventEmitter();
    child.stdin = new EventEmitter();
    child.stdin.write = jasmine.createSpy('stdin.write');
    child.kill = jasmine.createSpy('kill');
    spyOn(proc as any, '_spawnProcess').andCallFake((mode: string, args: string[]) => {
      spawnArgs = [mode, ...(args || [])];
      (proc as any)._proc = child;
    });
    await startFakeEngine();
  });

  afterEach(function () {
    if (savedTransport === undefined) {
      delete process.env.TERN_ENGINE_TRANSPORT;
    } else {
      process.env.TERN_ENGINE_TRANSPORT = savedTransport;
    }
    (proc as any)._closeSocket();
    if (conn) conn.destroy();
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('passes --socket, says hello and subscribes, then sends what was queued', async function () {
    proc.sync();
    expect(spawnArgs).toEqual(['sync', '--socket', socketPath]);
    proc.sendMessage({ type: 'wake-workers' });
    listening();

    const [hello, subscribe, wake] = await requests(3);
    expect(hello.method).toEqual('hello');
    expect(hello.params.protocol).toEqual(1);
    expect(hello.params.client).toMatch(/^tern-desktop\//);
    expect(subscribe.method).toEqual('subscribe');
    expect(subscribe.params).toEqual({ accounts: null, classes: null });
    expect(wake.method).toEqual('sync.wake');
    expect(wake.params).toEqual({ accountId: ACCOUNT_ID });
    expect(child.stdin.write).not.toHaveBeenCalled();
  });

  it('emits socket deltas and the stdout lines other than the listening one', async function () {
    const batches: string[][] = [];
    proc.on('deltas', (msgs: string[]) => batches.push(msgs));
    proc.sync();
    const secrets = JSON.stringify({
      modelClass: 'ProcessAccountSecretsUpdated',
      type: 'persist',
      modelJSONs: [{ id: ACCOUNT_ID }],
    });
    child.stdout.emit(
      'data',
      Buffer.from(`\nWaiting for Account JSON:\n${JSON.stringify({ listening: socketPath })}\n`)
    );
    child.stdout.emit('data', Buffer.from(`${secrets}\n`));
    await requests(2);

    const fromSocket = nextEvent(proc, 'deltas');
    const params = { accountId: ACCOUNT_ID, modelClass: 'Thread', type: 'persist', modelJSONs: [] };
    conn.write(
      `{"jsonrpc":"2.0","method":"delta","params":${JSON.stringify(params)}}\n` +
        `{ "jsonrpc": "2.0", "method": "delta", "params": ${JSON.stringify(params)} }\n`
    );
    const msgs = await fromSocket;

    expect(batches[0]).toEqual(['', 'Waiting for Account JSON:']);
    expect(batches[1]).toEqual([secrets]);
    expect(msgs.map((m) => JSON.parse(m))).toEqual([params, params]);
  });

  it('maps the stdin commands to methods and leaves the rest on stdin', async function () {
    proc.sync();
    listening();
    await requests(2);
    const task = { __cls: 'ChangeStarredTask', id: 't1' };
    proc.sendMessage({ type: 'queue-task', task });
    proc.sendMessage({ type: 'cancel-task', taskId: 't1' });
    proc.sendMessage({ type: 'need-bodies', ids: ['m1', 'm2'] });
    proc.sendMessage({ type: 'sync-calendar' });
    proc.sendMessage({ type: 'test-crash' });

    const sent = (await requests(6)).slice(2);
    expect(sent.map((r) => [r.method, r.params])).toEqual([
      ['task.queue', { accountId: ACCOUNT_ID, task }],
      ['task.cancel', { accountId: ACCOUNT_ID, taskId: 't1' }],
      ['bodies.fetch', { accountId: ACCOUNT_ID, ids: ['m1', 'm2'] }],
      ['calendar.sync', { accountId: ACCOUNT_ID }],
    ]);
    expect(new Set(sent.map((r) => r.id)).size).toEqual(4);
    expect(child.stdin.write).toHaveBeenCalledWith('{"type":"test-crash"}\n', 'utf-8');
  });

  it('kills the engine and closes with -2 when the socket drops', async function () {
    proc.sync();
    listening();
    await requests(2);
    const closes: any[] = [];
    proc.on('close', (e) => closes.push(e));
    const clientClosed = nextEvent((proc as any)._socket, 'close');
    conn.destroy();
    await clientClosed;

    // The engine gets a moment to exit on its own, which would carry its real exit code.
    expect(closes).toEqual([]);
    advanceClock(1000);
    expect(child.kill).toHaveBeenCalled();
    expect(closes.length).toEqual(1);
    expect(closes[0].code).toEqual(-2);
    child.emit('exit', null, 'SIGTERM');
    expect(closes.length).toEqual(1);
  });

  it('reports the exit code when the engine exits right after the socket drops', async function () {
    proc.sync();
    listening();
    await requests(2);
    const closes: any[] = [];
    proc.on('close', (e) => closes.push(e));
    const clientClosed = nextEvent((proc as any)._socket, 'close');
    conn.destroy();
    await clientClosed;
    child.stderr.emit('data', Buffer.from('panicked'));
    child.emit('exit', 101, null);
    advanceClock(1000);

    expect(child.kill).not.toHaveBeenCalled();
    expect(closes.length).toEqual(1);
    expect(closes[0].code).toEqual(101);
    expect(`${closes[0].error}`).toContain('panicked');
  });

  it('treats a disconnect notification as a close', async function () {
    spyOn(console, 'warn');
    proc.sync();
    listening();
    await requests(2);
    const closed = nextEvent(proc, 'close');
    conn.write('{"jsonrpc":"2.0","method":"disconnect","params":{"reason":"fell behind"}}\n');
    const exit = await closed;

    expect(exit.code).toEqual(-2);
    expect(`${exit.error}`).toContain('fell behind');
    expect(child.kill).toHaveBeenCalled();
  });

  it('does not report a close when the bridge killed the engine', async function () {
    proc.sync();
    listening();
    await requests(2);
    const closes: any[] = [];
    proc.on('close', (e) => closes.push(e));
    proc.kill();
    const clientClosed = nextEvent((proc as any)._socket, 'close');
    conn.destroy();
    await clientClosed;
    advanceClock(1000);
    expect(closes).toEqual([]);
    child.emit('exit', null, 'SIGTERM');
    expect(closes.map((e) => e.signal)).toEqual(['SIGTERM']);
  });

  it('stays on stdio with TERN_ENGINE_TRANSPORT=stdio', function () {
    process.env.TERN_ENGINE_TRANSPORT = 'stdio';
    proc.sync();
    expect(spawnArgs).toEqual(['sync']);
    proc.sendMessage({ type: 'wake-workers' });
    expect(child.stdin.write).toHaveBeenCalledWith('{"type":"wake-workers"}\n', 'utf-8');
  });

  it('stays on stdio for the C++ engine', function () {
    (proc as any).syncEngine = 'cpp';
    proc.sync();
    expect(spawnArgs).toEqual(['sync']);
    const deltas: string[][] = [];
    proc.on('deltas', (msgs: string[]) => deltas.push(msgs));
    child.stdout.emit('data', Buffer.from(`${JSON.stringify({ listening: socketPath })}\n`));
    expect(deltas).toEqual([[JSON.stringify({ listening: socketPath })]]);
  });
});
