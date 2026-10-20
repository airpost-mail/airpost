import { EventEmitter } from 'events';
import fs from 'fs';
import {
  EngineServeProcess,
  errorFromEngineResult,
  lastJSONResponse,
  MailsyncProcess,
  stripSecretKeys,
} from '../../src/engine/process';
import { FakeEngine, until } from '../support/fake-serve-engine';

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
  // mailsync's result line can carry a whole IMAP/SMTP conversation, so it can span more
  // than one 64KB chunk, and a server error or account name is often
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

  const errorFromRun = (stdout: string, code: number) => {
    const promise = proc._spawnAndWait('migrate');
    child.stdout.emit('data', Buffer.from(stdout, 'utf-8'));
    child.emit('close', code, null);
    return promise.then(
      () => {
        throw new Error('expected the run to reject');
      },
      (err) => err
    );
  };

  it('builds the error from the JSON result of a failed run', async function () {
    const err = await errorFromRun('{"error":"ErrorParse","error_service":"imap","log":"x"}', 1);
    expect(err.message).toContain('(IMAP)');
    expect(err.rawLog).toBe('x');
  });

  it('reports a crash as an unknown error, whatever the dump contains', async function () {
    const err = await errorFromRun('terminate called\n{"offline":true,"retryable":true', 134);
    expect(err.message).toContain('mailsync: 134');
    expect(err.isNetworkError).toBe(undefined);
  });
});

describe('the error of a failed account test', function () {
  const error = (result: any) => errorFromEngineResult(result, stripSecretKeys) as any;

  it('flags a failure the engine classified as this machine being offline', function () {
    expect(
      error({ error: 'ErrorConnection', error_service: 'imap', error_offline: true, log: '' })
        .isNetworkError
    ).toBe(true);
  });

  it('does not flag a failure the server itself reported', function () {
    expect(
      error({ error: 'ErrorAuthentication', error_service: 'imap', error_offline: false, log: '' })
        .isNetworkError
    ).toBe(false);
  });

  it("marks classified errors as the user's, except the ambiguous ones", function () {
    expect(error({ error: 'ErrorAuthentication', log: '' }).isUserError).toBe(true);
    expect(error({ error: 'ErrorParse', log: '' }).isUserError).toBe(false);
    expect(error({ error: 'Something else', log: '' }).isUserError).toBe(false);
  });

  it('appends the TLS advice and redacts secret keys in the log', function () {
    const err = error({
      error: 'ErrorCertificate',
      error_service: 'smtp',
      error_advice: 'Check the certificate.',
      log: '{"refresh_token":"abc\\"def"}',
    });
    expect(err.message).toContain('(SMTP) Check the certificate.');
    expect(err.errorAdvice).toBe('Check the certificate.');
    expect(err.errorService).toBe('smtp');
    expect(err.rawLog).toBe('{"refresh_token":"*********"}');
  });
});

describe('migrating', function () {
  it('runs import-legacy after migrate and launches anyway when it fails', async function () {
    const proc = new MailsyncProcess({ configDirPath: '/tmp', resourcePath: '/tmp' });
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

describe("the serve engine's socket", function () {
  const ACCOUNT_ID = 'acc1';
  let dir: string = null;
  let fake: FakeEngine = null;
  let proc: EngineServeProcess = null;

  const nextEvent = (emitter: EventEmitter, event: string) =>
    new Promise<any>((resolve) => emitter.once(event, resolve));

  const connected = async (n = 2) => {
    proc.serve();
    await until('hello and subscribe', () => fake.socket.length >= n);
  };

  beforeEach(async function () {
    // Short, unlike os.tmpdir() on macOS, so <configDir>/run/engine.sock is the path used.
    dir = fs.mkdtempSync('/tmp/tern-spec-');
    fake = new FakeEngine(dir);
    await fake.start();
    proc = new EngineServeProcess({ configDirPath: dir, resourcePath: dir });
    spyOn(proc as any, '_spawnProcess').andCallFake(function (mode: string, args: string[]) {
      this._proc = fake.spawn(mode, args);
    });
    spyOn(console, 'log');
  });

  afterEach(function () {
    proc._closeSocket();
    fake.stop();
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('passes --socket, says hello and subscribes, then sends what was queued', async function () {
    proc.serve();
    expect([fake.spawns[0].mode, ...fake.spawns[0].args]).toEqual([
      'serve',
      '--socket',
      fake.socketPath,
    ]);
    proc.wakeAll();
    await until('three requests', () => fake.socket.length >= 3);

    const [hello, subscribe, wake] = fake.socket;
    expect(hello.method).toEqual('hello');
    expect(hello.params.protocol).toEqual(1);
    expect(hello.params.client).toMatch(/^tern-desktop\//);
    expect(subscribe.method).toEqual('subscribe');
    expect(subscribe.params).toEqual({ accounts: null, classes: null });
    expect(wake.method).toEqual('sync.wake');
    expect(fake.stdin.map((r) => r.method)).toEqual(['account.list']);
  });

  it('keeps hello.features, answers request() and emits mail.new', async function () {
    fake.socketResults.hello = { protocol: 1, accounts: [], features: ['counts', 'mail-new'] };
    fake.socketErrors['tasks.list'] = { code: -32601, message: 'unknown method tasks.list' };
    const hello = nextEvent(proc, 'hello');
    proc.serve();
    expect((await hello).features).toEqual(['counts', 'mail-new']);
    expect(proc.features).toEqual(['counts', 'mail-new']);

    expect(await proc.request('counts.get', {})).toEqual({});
    let refused: any = null;
    await proc.request('tasks.list', {}).catch((e) => (refused = e));
    expect([refused.code, refused.message]).toEqual([-32601, 'unknown method tasks.list']);

    const mailNew = nextEvent(proc, 'mail-new');
    const params = { accountId: ACCOUNT_ID, threadId: 't1', messageId: 'm1' };
    fake.notify('mail.new', params);
    expect(await mailNew).toEqual(params);

    // A request still out when the connection ends is rejected.
    const pending: Promise<any> = proc.request('counts.get', {}).catch((e) => e);
    proc._closeSocket();
    expect((await pending).engineExited).toBe(true);
  });

  it('reads an engine without hello.features as having none', async function () {
    const hello = nextEvent(proc, 'hello');
    proc.serve();
    expect(proc.features).toBe(null);
    await hello;
    expect(proc.features).toEqual([]);
  });

  // The secrets deltas on stdout are in engine-host-spec.
  it('emits the socket deltas, however the envelope is spaced', async function () {
    await connected();
    const fromSocket = nextEvent(proc, 'deltas');
    const params = { accountId: ACCOUNT_ID, modelClass: 'Thread', type: 'persist', modelJSONs: [] };
    fake.conn.write(
      `{"jsonrpc":"2.0","method":"delta","params":${JSON.stringify(params)}}\n` +
        `{ "jsonrpc": "2.0", "method": "delta", "params": ${JSON.stringify(params)} }\n`
    );
    const msgs = await fromSocket;

    expect(msgs.map((m) => JSON.parse(m))).toEqual([params, params]);
  });

  it('maps the account commands to socket methods and test-crash to stdin', async function () {
    spyOn(console, 'warn');
    await connected();
    const task = { __cls: 'ChangeStarredTask', id: 't1' };
    proc.sendToAccount(ACCOUNT_ID, { type: 'queue-task', task });
    proc.sendToAccount(ACCOUNT_ID, { type: 'cancel-task', taskId: 't1' });
    proc.sendToAccount(ACCOUNT_ID, { type: 'need-bodies', ids: ['m1', 'm2'] });
    proc.sendToAccount(ACCOUNT_ID, { type: 'sync-calendar' });
    proc.sendToAccount(ACCOUNT_ID, { type: 'test-crash' });

    await until('the commands', () => fake.socket.length >= 6);
    const sent = fake.socket.slice(2);
    expect(sent.map((r) => [r.method, r.params])).toEqual([
      ['task.queue', { accountId: ACCOUNT_ID, task }],
      ['task.cancel', { accountId: ACCOUNT_ID, taskId: 't1' }],
      ['bodies.fetch', { accountId: ACCOUNT_ID, ids: ['m1', 'm2'] }],
      ['calendar.sync', { accountId: ACCOUNT_ID }],
    ]);
    expect(new Set(sent.map((r) => r.id)).size).toEqual(4);
    await until('test.crash', () => fake.stdinOf('test.crash').length === 1);
    expect(fake.stdinOf('test.crash')[0].params).toEqual({ accountId: ACCOUNT_ID });
  });

  it('kills the engine and closes with -2 when the socket drops', async function () {
    spyOn(console, 'warn');
    await connected();
    const closes: any[] = [];
    proc.on('close', (e) => closes.push(e));
    const clientClosed = nextEvent(proc._socket, 'close');
    fake.conn.destroy();
    await clientClosed;

    // The engine gets a moment to exit on its own, which would carry its real exit code.
    expect(closes).toEqual([]);
    advanceClock(1000);
    expect(fake.child.kill).toHaveBeenCalled();
    expect(closes.length).toEqual(1);
    expect(closes[0].code).toEqual(-2);
    fake.child.emit('exit', null, 'SIGTERM');
    expect(closes.length).toEqual(1);
  });

  it('reports the exit code when the engine exits right after the socket drops', async function () {
    await connected();
    const closes: any[] = [];
    proc.on('close', (e) => closes.push(e));
    const clientClosed = nextEvent(proc._socket, 'close');
    fake.conn.destroy();
    await clientClosed;
    fake.child.stderr.emit('data', Buffer.from('panicked'));
    fake.child.emit('exit', 101, null);
    advanceClock(1000);

    expect(fake.child.kill).not.toHaveBeenCalled();
    expect(closes.length).toEqual(1);
    expect(closes[0].code).toEqual(101);
    expect(`${closes[0].error}`).toContain('panicked');
  });

  it('treats a disconnect notification as a close', async function () {
    spyOn(console, 'warn');
    await connected();
    const closed = nextEvent(proc, 'close');
    fake.notify('disconnect', { reason: 'fell behind' });
    const exit = await closed;

    expect(exit.code).toEqual(-2);
    expect(`${exit.error}`).toContain('fell behind');
    expect(fake.child.kill).toHaveBeenCalled();
  });

  it('does not report a socket failure when the engine was killed', async function () {
    await connected();
    const closes: any[] = [];
    proc.on('close', (e) => closes.push(e));
    proc.kill();
    const clientClosed = nextEvent(proc._socket, 'close');
    fake.conn.destroy();
    await clientClosed;
    advanceClock(1000);
    expect(closes).toEqual([]);
    fake.child.emit('exit', null, 'SIGTERM');
    expect(closes.map((e) => e.signal)).toEqual(['SIGTERM']);
  });
});
