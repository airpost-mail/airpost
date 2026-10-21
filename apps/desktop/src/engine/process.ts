/* eslint global-require: 0 */

/*
Warning! This file is imported from the main process as well as the renderer process
*/
import { spawn, ChildProcess } from 'child_process';
import crypto from 'crypto';
import { StringDecoder } from 'string_decoder';
import path from 'path';
import { EventEmitter } from 'events';
import fs from 'fs';
import net from 'net';
import { localized } from '../intl/localize';
import { developerToolsAvailable } from '../platform/developer-tools';
import {
  AccountAddResult,
  AccountChangedParams,
  AccountRemoveResult,
  AccountStateParams,
  CredentialsImportParams,
  CredentialsImportResult,
  DisconnectParams,
  HelloParams,
  HelloResult,
  MailNewParams,
  SettingsValues,
  MAX_LINE_BYTES,
  Method,
  PROTOCOL_VERSION,
  RequestId,
  requestForCommand,
  ResetDatabaseResult,
  RpcResponse,
  RpcError,
  serveSocketPath,
  SetLogLevelResult,
  SubscribeParams,
  deltaParamsText,
} from './protocol';

// account.reset / engine.resetDatabase: whether plugin metadata (send-later, snoozes,
// reminders) survives. Removing an account passes false, so an account added again with the
// same id doesn't get the old schedules back.
export interface ResetOptions {
  keepMetadata?: boolean;
}

// `code` is null when a signal ended the engine.
export interface EngineProcessExit {
  code: number | null;
  error?: Error | null;
  signal: string | null;
}

export const LocalizedErrorStrings = {
  ErrorConnection: localized(
    'Connection Error - Unable to connect to the server / port you provided.'
  ),
  ErrorInvalidAccount: localized(
    'This account is invalid or Tern could not find the Inbox or All Mail folder. %@',
    'https://support.google.com/mail/answer/7126229'
  ),
  ErrorTLSNotAvailable: localized('TLS Not Available'),
  ErrorParse: localized('Parsing Error'),
  ErrorCertificate: localized('Certificate Error'),
  ErrorAuthentication: localized('Authentication Error - Check your username and password.'),
  ErrorGmailIMAPNotEnabled: localized(
    'Gmail IMAP is not enabled. Visit Gmail settings to turn it on.'
  ),
  ErrorGmailExceededBandwidthLimit: localized('Gmail bandwidth exceeded. Please try again later.'),
  ErrorGmailTooManySimultaneousConnections: localized(
    'There are too many active connections to your Gmail account. Please try again later.'
  ),
  ErrorMobileMeMoved: localized('MobileMe has moved.'),
  ErrorYahooUnavailable: localized('Yahoo is unavailable.'),
  ErrorNonExistantFolder: localized('Sorry, this folder does not exist.'),
  ErrorStartTLSNotAvailable: localized('StartTLS is not available.'),
  ErrorGmailApplicationSpecificPasswordRequired: localized(
    'A Gmail application-specific password is required.'
  ),
  ErrorOutlookLoginViaWebBrowser: localized(
    'The Outlook server said you must sign in via a web browser.'
  ),
  ErrorNeedsConnectToWebmail: localized('The server said you must sign in via your webmail.'),
  ErrorNoValidServerFound: localized('No valid server found.'),
  ErrorAuthenticationRequired: localized('Authentication required.'),

  // sending related
  ErrorSendMessageNotAllowed: localized('Sending is not enabled for this account.'),
  ErrorSendMessageIllegalAttachment: localized(
    'The message contains an illegal attachment that is not allowed by the server.'
  ),
  ErrorYahooSendMessageSpamSuspected: localized(
    "The message has been blocked by Yahoo's outbound spam filter."
  ),
  ErrorYahooSendMessageDailyLimitExceeded: localized(
    'The message has been blocked by Yahoo - you have exceeded your daily sending limit.'
  ),
  ErrorNoSender: localized('The message has been blocked because no sender is configured.'),
  ErrorInvalidRelaySMTP: localized(
    'The SMTP server would not relay a message. You may need to authenticate.'
  ),
  ErrorNoImplementedAuthMethods: localized(
    'Sorry, your SMTP server does not support basic username / password authentication.'
  ),
};

// LocalizedErrorStrings codes that, despite being "classified" by the engine,
// can also indicate a genuine engine defect rather than a server/user
// condition - a malformed-response parser regression. These should keep
// reaching error reporting instead of being treated as expected,
// user-actionable failures.
const AMBIGUOUS_ENGINE_ERRORS = new Set(['ErrorParse']);

/*
Extracts the engine's JSON result from what it wrote to stdout.

The result is one JSON object, but the end of the stream is not reliably that object. It
arrives without a trailing newline, the engine prints human-readable progress lines before it
(`Running Setup`), and on Linux it can emit a diagnostic - a missing libtidy is the common
one - at any point. Parsing the last line of stdout and stderr combined turns any of those
into "an unknown error has occurred tern-engine: 0", so scan stdout backwards for the last
line that actually parses.
*/
export function lastJSONResponse(stdout: string): any | null {
  const lines = stdout.split('\n');
  for (let i = lines.length - 1; i >= 0; i--) {
    const line = lines[i].trim();
    if (!line.startsWith('{')) continue;
    try {
      return JSON.parse(line);
    } catch (err) {
      // Not the result line - keep looking further back.
    }
  }
  return null;
}

// The Rust engine, unpacked from the asar by build/build.js.
export function engineBinaryPath(resourcePath: string) {
  return path.join(resourcePath, 'tern-engine').replace('app.asar', 'app.asar.unpacked');
}

// A binary shipped next to tern-engine: the env override (a source checkout), else the bundled
// one; null when neither exists.
function bundledBinaryPath(resourcePath: string, name: string, envVar: string): string | null {
  const fromEnv = process.env[envVar];
  if (fromEnv) return fs.existsSync(fromEnv) ? fromEnv : null;
  const file = process.platform === 'win32' ? `${name}.exe` : name;
  const bundled = path.join(resourcePath, file).replace('app.asar', 'app.asar.unpacked');
  return fs.existsSync(bundled) ? bundled : null;
}

// The `tern` CLI: TERN_BIN, else the bundled one.
export function ternBinaryPath(resourcePath: string): string | null {
  return bundledBinaryPath(resourcePath, 'tern', 'TERN_BIN');
}

// tern-mcp (apps/mcp): TERN_MCP_BIN, else the bundled one.
export function ternMcpBinaryPath(resourcePath: string): string | null {
  return bundledBinaryPath(resourcePath, 'tern-mcp', 'TERN_MCP_BIN');
}

// tern-gateway (apps/gateway): TERN_GATEWAY_BIN, else the bundled one.
export function ternGatewayBinaryPath(resourcePath: string): string | null {
  return bundledBinaryPath(resourcePath, 'tern-gateway', 'TERN_GATEWAY_BIN');
}

// The built web client (apps/web, `npm run build:desktop`) tern-gateway serves:
// TERN_GATEWAY_WEB_ROOT, else the `tern-web` folder build.js ships unpacked.
export function ternWebRootPath(resourcePath: string): string | null {
  const fromEnv = process.env.TERN_GATEWAY_WEB_ROOT;
  const bundled = path.join(resourcePath, 'tern-web').replace('app.asar', 'app.asar.unpacked');
  const dir = fromEnv || bundled;
  return fs.existsSync(path.join(dir, 'index.html')) ? dir : null;
}

// Unpackaged runs (`make desktop`, `npm start`, specs) keep the engine's secrets in a file
// keyed by this, not the keychain: an unsigned engine rebuilt every day would make the
// keychain ask each time (Signing).
export const DEV_ENGINE_KEY_FILE = 'engine-dev.key';

export function devEngineKeyFile(configDirPath: string, resourcePath: string) {
  if (resourcePath.includes('app.asar')) return null;
  return path.join(configDirPath, DEV_ENGINE_KEY_FILE);
}

// `hello.build` of an engine running this binary: `sha256:<hex>` of the file.
export function engineBuildId(binaryPath: string): Promise<string | null> {
  return new Promise((resolve) => {
    const hash = crypto.createHash('sha256');
    const stream = fs.createReadStream(binaryPath);
    stream.on('error', () => resolve(null));
    stream.on('data', (chunk) => hash.update(chunk));
    stream.on('end', () => resolve(`sha256:${hash.digest('hex')}`));
  });
}

/*
The Error a failed `account.test` (or a one-shot mode's JSON result) stands for, with the
fields onboarding reads: `rawLog`, `errorAdvice`, `errorService`, `isNetworkError` and
`isUserError`. `stripSecrets` redacts the transcript.
*/
export function errorFromEngineResult(response: any, stripSecrets: (text: string) => string) {
  const isRecognizedEngineError = Object.prototype.hasOwnProperty.call(
    LocalizedErrorStrings,
    response.error
  );
  let msg = isRecognizedEngineError
    ? LocalizedErrorStrings[response.error as keyof typeof LocalizedErrorStrings]
    : response.error;
  if (response.error_service) {
    msg = `${msg} (${response.error_service.toUpperCase()})`;
  }
  // Set only for a rejected TLS handshake today; arrives unlocalized.
  if (response.error_advice) {
    msg = `${msg} ${response.error_advice}`;
  }
  const error = new Error(msg);
  (error as any).rawLog = stripSecrets(response.log);
  (error as any).errorAdvice = response.error_advice || null;
  (error as any).errorService = response.error_service || null;
  // Set when the engine classified the failure as this machine being unable to reach the
  // network, rather than a server refusing or rejecting us.
  (error as any).isNetworkError = response.error_offline === true;
  // Errors the engine explicitly classified (bad credentials, unreachable server,
  // TLS/certificate problems, provider-side rate limits, etc.) describe the mail server or
  // the user's settings, not a bug - except for the ambiguous codes above, which we still
  // want reported if they occur. Callers that report unexpected errors to Sentry (e.g.
  // oauth-signin-page's error handler) check this flag to avoid flooding error tracking
  // with expected, user-actionable failures.
  (error as any).isUserError =
    isRecognizedEngineError && !AMBIGUOUS_ENGINE_ERRORS.has(response.error);
  return error;
}

// Redacts the values of known secret keys in a log or error text (e.g.
// "refresh_token":"abc"), which catches secrets the engine rotated and the client has not
// seen yet.
export function stripSecretKeys(text: string) {
  if (!text) return text;
  let out = text;
  const SENSITIVE_JSON_KEYS = ['refresh_token', 'access_token', 'imap_password', 'smtp_password'];
  for (const key of SENSITIVE_JSON_KEYS) {
    // Match "key":"<anything-but-unescaped-quote>" allowing escaped quotes inside.
    const re = new RegExp(`("${key}"\\s*:\\s*")(?:\\\\.|[^"\\\\])+(")`, 'g');
    out = out.replace(re, '$1*********$2');
  }
  return out;
}

// A crashed engine closes its socket as it exits. How long a dropped socket waits for the exit
// event, so the close the bridge sees carries the real exit code and stderr.
const SOCKET_CLOSE_GRACE_MS = 1000;

// The temp-dir fallback for socket paths: ours alone, 0700.
function ensurePrivateDir(dir: string) {
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  const stat = fs.lstatSync(dir);
  if (!stat.isDirectory()) {
    throw new Error(`${dir} is not a directory`);
  }
  if (typeof process.getuid === 'function' && stat.uid !== process.getuid()) {
    throw new Error(`${dir} belongs to another user`);
  }
  if ((stat.mode & 0o077) !== 0) {
    fs.chmodSync(dir, 0o700);
  }
}

/*
The engine binary: the one-shot `migrate` and `import-legacy` the main process runs before
anything else, and (EngineServeProcess) the `serve` engine every account runs in.
*/
export class EngineProcess extends EventEmitter {
  _proc: ChildProcess | null = null;
  _win: Electron.BrowserWindow | null = null;

  // The socket connection (EngineServeProcess). `_socketQueue` holds requests sent before the
  // engine bound its socket; it is null once connected and after the connection ends.
  _socketPath: string | null = null;
  _socket: net.Socket | null = null;
  _socketQueue: string[] | null = null;
  _socketRequests = new Map<RequestId, string>();
  // Socket requests whose answer someone awaits (request()), by id.
  _socketCalls = new Map<
    RequestId,
    { resolve: (result: any) => void; reject: (error: Error) => void }
  >();
  // `hello.features` of the engine on the socket; null until hello answers.
  features: string[] | null = null;
  _nextRequestId = 1;
  _killed = false;
  // What the socket connection subscribes to. The main process's supervisor takes no model
  // class, which still brings it every account.state (src/engine/supervisor/supervisor.ts).
  subscription: SubscribeParams = { accounts: null, classes: null };

  verbose?: boolean;
  resourcePath: string;
  configDirPath: string;
  binaryPath: string;

  constructor({
    configDirPath,
    resourcePath,
    verbose,
  }: {
    configDirPath: string;
    resourcePath: string;
    verbose?: boolean;
  }) {
    super();
    this.verbose = verbose;
    this.resourcePath = resourcePath;
    this.configDirPath = configDirPath;
    this.binaryPath = engineBinaryPath(resourcePath);
  }

  _showStatusWindow(mode: string) {
    if (this._win) return;

    // Only main runs migrate(); this file loads in windows too, where BrowserWindow is undefined.
    const { BrowserWindow }: typeof Electron = require('electron');

    const win = new BrowserWindow({
      width: 350,
      height: 108,
      show: false,
      center: true,
      autoHideMenuBar: true,
      resizable: false,
      minimizable: false,
      maximizable: false,
      closable: false,
      fullscreenable: false,
      webPreferences: {
        nodeIntegration: false,
        javascript: false,
        contextIsolation: false,
        devTools: developerToolsAvailable(),
      },
    });
    this._win = win;
    win.setContentSize(350, 90);
    win.once('ready-to-show', () => {
      win.show();
    });
    win.loadURL(`file://${this.resourcePath}/static/db-${mode}.html`);
  }

  _closeStatusWindow() {
    const win = this._win;
    if (!win) return;
    win.removeAllListeners('ready-to-show');
    win.setClosable(true);
    win.hide();
    setTimeout(() => {
      // don't know why this timeout is necessary but the app becomes unable to
      // load Electron modules in the main process if we close immediately.
      if (!win.isDestroyed()) win.close();
      this._win = null;
    });
  }

  _spawnProcess(mode: string, extraArgs: string[] = []) {
    // Build on a null-prototype object so the spawned engine inherits only
    // genuine environment variables, not properties a prototype-pollution bug
    // elsewhere in the process could have added to Object.prototype. See
    // GHSA-gjr7-3mj2-cr54.
    // The OAuth clients are the engine's own (MS_GMAIL_CLIENT_ID and the like override them).
    const env = Object.assign(Object.create(null), process.env, {
      CONFIG_DIR_PATH: this.configDirPath,
    });
    const devKeyFile = devEngineKeyFile(this.configDirPath, this.resourcePath);
    if (devKeyFile && !env.TERN_ENGINE_KEY_FILE) env.TERN_ENGINE_KEY_FILE = devKeyFile;
    const args = [`--mode`, mode];
    if (this.verbose) {
      args.push('--verbose');
    }
    args.push(...extraArgs);
    const proc = spawn(this.binaryPath, args, { env });
    this._proc = proc;

    /* Allow us to buffer up to 1MB on stdin instead of 16k. This is necessary
    because some tasks (creating replies to drafts, etc.) can be gigantic amounts
    of HTML, many tasks can be created at once, etc, and we don't want to kill
    the channel. */
    if (proc.stdin) {
      proc.stdin.setDefaultEncoding('utf-8');
      (proc.stdin as any).highWaterMark = 1024 * 1024;
    }
  }

  // The process _spawnProcess just started (specs replace _spawnProcess, so it is read back
  // from `_proc`).
  _spawned(): ChildProcess {
    if (!this._proc) throw new Error('tern-engine was not spawned');
    return this._proc;
  }

  // Redacts known secrets from a log or error text before it's surfaced to the UI, error
  // reporting, or thrown errors. EngineServeProcess also replaces the secrets of the accounts
  // it was given.
  _stripSecrets(text: string) {
    return stripSecretKeys(text);
  }

  _spawnAndWait(mode: string, { onData }: { onData?: (data: any) => void } = {}) {
    return new Promise<{ response: any; buffer: Buffer }>((resolve, reject) => {
      this._spawnProcess(mode);
      const proc = this._spawned();
      // `bothChunks` is both streams together and is what we show the user when something
      // goes wrong; `outChunks` is stdout alone, which is the only place the JSON result
      // appears. Chunks are concatenated as bytes and decoded once at the end: `a += b` on
      // two Buffers coerces both through toString(), which decodes every chunk on its own
      // and mangles any multi-byte character that straddles a chunk boundary.
      const bothChunks: Buffer[] = [];
      const outChunks: Buffer[] = [];

      if (proc.stdout) {
        proc.stdout.on('data', (data: Buffer) => {
          bothChunks.push(data);
          outChunks.push(data);
          if (onData) onData(data);
        });
      }
      if (proc.stderr) {
        proc.stderr.on('data', (data: Buffer) => {
          bothChunks.push(data);
          if (onData) onData(data);
        });
      }

      proc.on('error', (err: Error) => {
        reject(err);
      });

      proc.on('close', (code, signal) => {
        const buffer = Buffer.concat(bothChunks);
        try {
          const response = lastJSONResponse(Buffer.concat(outChunks).toString('utf-8'));
          if (!response) {
            // If the engine executable itself failed to run, the logs are not JSON
            // and may contain system errors (shared library issues, etc). Include this
            // in the logs so users can fix on their own or report detailed bugs.
            const rawLog = this._stripSecrets(buffer.toString());
            return reject(this._buildCrashError(code, signal, rawLog));
          }

          if (code === 0) {
            resolve({ response, buffer });
          } else {
            return reject(errorFromEngineResult(response, (t) => this._stripSecrets(t)));
          }
        } catch (err) {
          const rawLog = this._stripSecrets(buffer.toString());
          return reject(this._buildCrashError(code, signal, rawLog));
        }
      });
    });
  }

  // Called when the engine child process wrote no parseable JSON result, meaning it
  // crashed outright or never ran: a panic, the executable-path check that exits 2, a
  // missing shared library. The raw log is part of the message because
  // those are diagnosable by the user or by whoever reads the bug report. A process
  // terminated by a signal has a null `code`, which would otherwise read "tern-engine: null".
  _buildCrashError(code: number | null, signal: NodeJS.Signals | null, rawLog: string) {
    const exitDescription = signal ? `signal ${signal}` : `${code}`;
    const error = new Error(
      `${localized(`An unknown error has occurred`)} tern-engine: ${exitDescription}. ${rawLog}`
    );
    (error as any).rawLog = rawLog;
    return error;
  }

  kill() {
    console.warn('Terminating tern-engine...');
    this._killed = true;
    this._proc && this._proc.kill();
  }

  _clientName() {
    let version = 'unknown';
    try {
      const AppEnv = (global as any).AppEnv;
      version = AppEnv
        ? AppEnv.getVersion()
        : JSON.parse(fs.readFileSync(path.join(this.resourcePath, 'package.json')).toString())
            .version;
    } catch (err) {
      // the version only appears in the engine's log
    }
    return `tern-desktop/${version}`;
  }

  // Sends a request over the socket. Returns its id, or null when it was dropped.
  _sendRequest(method: string, params: unknown): RequestId | null {
    const id = this._nextRequestId++;
    const line = `${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`;
    if (this._socket && !this._socket.destroyed) {
      this._socketRequests.set(id, method);
      this._socket.write(line, 'utf-8');
    } else if (this._socketQueue) {
      this._socketRequests.set(id, method);
      this._socketQueue.push(line);
    } else {
      console.warn(`Not connected to the sync engine; dropped ${method}`);
      return null;
    }
    return id;
  }

  // A socket request whose result the caller needs. Rejects with an EngineRequestError when
  // the engine refuses it, or with `engineExited` when the connection ends first.
  request<R = unknown>(method: string, params: unknown): Promise<R> {
    const id = this._sendRequest(method, params);
    if (id === null) {
      return Promise.reject(
        new EngineRequestError('Not connected to the sync engine', { engineExited: true })
      );
    }
    return new Promise<R>((resolve, reject) => this._socketCalls.set(id, { resolve, reject }));
  }

  _closeSocket() {
    this._socketQueue = null;
    this._socketRequests.clear();
    const calls = [...this._socketCalls.values()];
    this._socketCalls.clear();
    for (const { reject } of calls) {
      reject(new EngineRequestError('The sync engine connection ended', { engineExited: true }));
    }
    if (this._socket) {
      // A close we asked for is not a failure.
      this._socket.removeAllListeners();
      this._socket.on('error', () => {});
      this._socket.destroy();
      this._socket = null;
    }
  }

  /*
  Connects to the engine once it has bound its socket, says hello, subscribes to every
  delta and emits them as 'deltas' in the stdout shape the bridge reads: the delta object, with
  accountId added. `fail` ends the process with close code -2.
  */
  _connectSocket(listeningPath: string, fail: (error: Error, opts?: { now?: boolean }) => void) {
    if (this._socket || !this._socketQueue) return;
    const socket = net.createConnection(listeningPath);
    socket.setEncoding('utf8');
    this._socket = socket;

    // Writes before 'connect' are buffered by the socket. The engine reads requests in order,
    // so hello is handled before anything after it.
    const queued = this._socketQueue;
    this._socketQueue = null;
    this._sendRequest(Method.Hello, {
      protocol: PROTOCOL_VERSION,
      client: this._clientName(),
    } as HelloParams);
    this._sendRequest(Method.Subscribe, this.subscription);
    for (const line of queued) {
      socket.write(line, 'utf-8');
    }

    let inBuffer = '';
    socket.on('data', (chunk: string) => {
      inBuffer += chunk;
      if (chunk.indexOf('\n') === -1) {
        if (inBuffer.length > MAX_LINE_BYTES) {
          inBuffer = '';
          fail(new Error('The sync engine sent a line over the size limit'), { now: true });
        }
        return;
      }
      const lines = inBuffer.split('\n');
      inBuffer = lines.pop() ?? '';
      const deltas: string[] = [];
      let failure: Error | null = null;
      for (const line of lines) {
        failure = this._onSocketLine(line, deltas) || failure;
      }
      if (deltas.length) {
        this.emit('deltas', deltas);
      }
      if (failure) {
        fail(failure, { now: true });
      }
    });
    socket.on('error', (err: Error) => fail(err));
    socket.on('close', () => fail(new Error('The sync engine closed its socket')));
  }

  // Adds a delta line to `deltas`, or returns an error that ends the connection.
  _onSocketLine(line: string, deltas: string[]): Error | null {
    if (!line) return null;
    const params = deltaParamsText(line);
    if (params !== null) {
      deltas.push(params);
      return null;
    }
    let msg = null;
    try {
      msg = JSON.parse(line);
    } catch (err) {
      console.warn(`Sync engine sent a non-JSON line: ${this._stripSecrets(line)}`);
      return null;
    }
    if (msg.method === Method.Delta) {
      deltas.push(JSON.stringify(msg.params));
      return null;
    }
    if (msg.method === Method.AccountState) {
      this.emit('account-state', msg.params as AccountStateParams);
      return null;
    }
    if (msg.method === Method.MailNew) {
      this.emit('mail-new', msg.params as MailNewParams);
      return null;
    }
    if (msg.method === Method.AccountChanged) {
      this.emit('account-changed', msg.params as AccountChangedParams);
      return null;
    }
    if (msg.method === Method.SettingsChanged) {
      this.emit('settings-changed', msg.params as SettingsValues);
      return null;
    }
    if (msg.method === Method.Disconnect) {
      const { reason } = (msg.params || {}) as DisconnectParams;
      console.warn(`Sync engine dropped the connection: ${reason}`);
      return new Error(`The sync engine dropped the connection: ${reason}`);
    }
    if (msg.method === undefined && msg.id !== undefined) {
      const response = msg as RpcResponse;
      const id = response.id;
      const method = id === null ? undefined : this._socketRequests.get(id);
      const call = id === null ? undefined : this._socketCalls.get(id);
      if (id !== null) {
        this._socketRequests.delete(id);
        this._socketCalls.delete(id);
      }
      if (response.error) {
        const { code, message, data } = response.error;
        if (call) {
          call.reject(new EngineRequestError(message, { code, data }));
          return null;
        }
        console.warn(
          `Sync engine refused ${method || `request ${response.id}`}: ${message} (${code})`,
          data
        );
        if (method === Method.Hello) {
          return new Error(`The sync engine refused hello: ${message}`);
        }
        return null;
      }
      if (call) {
        call.resolve(response.result);
      } else if (method === Method.Hello) {
        const hello = (response.result || {}) as HelloResult;
        this.features = Array.isArray(hello.features) ? hello.features : [];
        this.emit('hello', hello);
      }
    }
    return null;
  }

  async migrate() {
    try {
      console.log('Running database migrations');
      const { buffer } = await this._spawnAndWait('migrate', {
        onData: (data) => {
          const str = data.toString().toLowerCase();
          if (str.includes('running migration')) this._showStatusWindow('migration');
        },
      });
      console.log(this._stripSecrets(buffer.toString()));
      // Import while the status window is still up: closing it must be the last step before
      // the main window opens, or the main process stalls with no window (see
      // _closeStatusWindow).
      await this._importLegacy();
      this._closeStatusWindow();
    } catch (err) {
      this._closeStatusWindow();
      throw err;
    }
  }

  // Carries local drafts, snoozes, reminders and send-later over from the C++ database that
  // the Rust engine's migrate moved aside. A no-op
  // without one or once imported, so it runs on every launch; a failure is logged and the
  // launch continues, since the account processes do not depend on it.
  async _importLegacy() {
    try {
      const { buffer } = await this._spawnAndWait('import-legacy');
      console.log(this._stripSecrets(buffer.toString()));
    } catch (err) {
      console.error(`tern-engine --mode import-legacy failed: ${err}`);
    }
  }
}

// How long `engine.shutdown` may take before the engine is killed. The engine stops every
// account at once and waits up to 5 s for each one's threads.
export const ENGINE_SHUTDOWN_GRACE_MS = 8000;

// A request on serve mode's stdin that the engine refused (`code`, `data`), or that it never
// answered because it exited (`engineExited`).
export class EngineRequestError extends Error {
  code: number | null;
  data: unknown;
  engineExited: boolean;

  constructor(
    message: string,
    {
      code = null,
      data,
      engineExited = false,
    }: { code?: number | null; data?: unknown; engineExited?: boolean } = {}
  ) {
    super(message);
    this.code = code;
    this.data = data;
    this.engineExited = engineExited;
  }
}

/*
The Rust engine's `serve` mode: one process for every account.
stdin and stdout are the parent channel: JSON-RPC requests for account.add,
account.remove, account.test, account.reset, engine.resetDatabase, engine.setLogLevel and
engine.shutdown, their responses, and the accounts'
ProcessAccountSecretsUpdated deltas, so credentials never touch the socket. Everything else
goes over the socket, connected once the first stdin response shows the engine has bound it.

Emits 'ready' once the engine has bound its socket, 'hello' (HelloResult) once the socket is
greeted, 'deltas' (delta params as JSON text, the shape the bridge reads), 'account-state'
(AccountStateParams), 'mail-new' (MailNewParams), 'settings-changed' (SettingsValues), and
'close' (EngineProcessExit) once, when the engine exits or its socket fails.
*/
export class EngineServeProcess extends EngineProcess {
  _parentRequests = new Map<
    number,
    { method: string; resolve: (result: any) => void; reject: (error: Error) => void }
  >();
  _nextParentId = 1;
  _closed = false;
  _shutdownTimer: ReturnType<typeof setTimeout> | undefined = undefined;
  _startupError: string | null = null;
  // Joined a detached engine (attach()) rather than spawned one.
  _attached = false;
  // When set, serve() joins the detached engine on this socket instead of spawning one.
  attachTo: string | null = null;
  // The secrets of every account added, redacted from what the engine reports. Kept after a
  // remove, since a late error may still carry them.
  _secretValues = new Set<string>();

  serve() {
    if (this.attachTo) {
      this.attach(this.attachTo);
      return;
    }
    const location = serveSocketPath(this.configDirPath);
    if (!location) {
      throw new Error('No socket path short enough for the sync engine');
    }
    if (location.ownDir) {
      ensurePrivateDir(location.ownDir);
    }
    const socketPath = location.socketPath;
    this._socketPath = socketPath;
    this._socketQueue = [];
    this._spawnProcess('serve', ['--socket', socketPath]);
    const proc = this._spawned();

    const decoder = new StringDecoder('utf8');
    let outBuffer = '';
    let errBuffer = '';
    let socketCloseTimer: ReturnType<typeof setTimeout> | undefined;

    const finish = (exit: EngineProcessExit) => {
      if (this._closed) return;
      this._closed = true;
      clearTimeout(socketCloseTimer);
      clearTimeout(this._shutdownTimer);
      this._closeSocket();
      const gone = new EngineRequestError('The sync engine exited', { engineExited: true });
      for (const { reject } of this._parentRequests.values()) {
        reject(gone);
      }
      this._parentRequests.clear();
      this.emit('close', exit);
    };

    // A dropped socket gets a moment for the real exit to arrive, then the engine is killed
    // and the close reports -2.
    const failSocket = (error: Error, { now }: { now?: boolean } = {}) => {
      if (this._closed || this._killed) return;
      const kill = () => {
        if (this._closed || this._killed) return;
        console.warn(`Sync engine socket failed: ${error}`);
        proc.kill();
        finish({ code: -2, error, signal: null });
      };
      if (now) {
        kill();
      } else if (!socketCloseTimer) {
        socketCloseTimer = setTimeout(kill, SOCKET_CLOSE_GRACE_MS);
      }
    };

    if (proc.stdout) {
      proc.stdout.on('data', (data: Buffer) => {
        outBuffer += decoder.write(data);
        if (outBuffer.indexOf('\n') === -1) return;
        const lines = outBuffer.split('\n');
        outBuffer = lines.pop() ?? '';
        const deltas: string[] = [];
        for (const line of lines) {
          this._onParentLine(line, deltas);
        }
        if (deltas.length) {
          this.emit('deltas', deltas);
        }
      });
    }
    if (proc.stderr) {
      proc.stderr.on('data', (data: Buffer) => {
        errBuffer += data.toString();
        if (errBuffer.length > 100 * 1024) {
          errBuffer = errBuffer.slice(-100 * 1024);
        }
      });
    }
    // Never re-emitted as 'error' on `this`: an EventEmitter with no 'error' listener throws.
    proc.on('error', (error: Error) => {
      console.log(`Sync engine exited with ${error}`);
      finish({ code: -1, error, signal: null });
    });
    if (proc.stdin) {
      proc.stdin.on('error', (error: Error) => {
        if (this._closed) return;
        proc.kill();
        finish({ code: -2, error, signal: null });
      });
    }
    const onExit = (code: number | null, signal: string | null) => {
      const text = errBuffer || this._startupError;
      finish({ code, error: text ? new Error(this._stripSecrets(text)) : null, signal });
    };
    proc.on('close', onExit);
    proc.on('exit', onExit);

    // stdin is read only once the socket is bound, so this answer means it can be connected.
    this._parentCall(Method.AccountList, {}).then(
      () => {
        this._connectSocket(socketPath, failSocket);
        this.emit('ready');
      },
      () => {}
    );
  }

  _stripSecrets(text: string) {
    let out = super._stripSecrets(text);
    if (!out) return out;
    for (const v of this._secretValues) {
      out = out.replaceAll(v, '*********');
    }
    return out;
  }

  _onParentLine(line: string, deltas: string[]) {
    if (!line.trim()) return;
    const params = deltaParamsText(line);
    if (params !== null) {
      deltas.push(params);
      return;
    }
    let msg = null;
    try {
      msg = JSON.parse(line);
    } catch (err) {
      console.warn(`Sync engine wrote a non-JSON line: ${this._stripSecrets(line)}`);
      return;
    }
    if (msg.jsonrpc === '2.0' && msg.method === undefined) {
      const pending = msg.id !== null ? this._parentRequests.get(msg.id) : undefined;
      if (!pending) {
        console.warn(`Sync engine sent an unexpected response: ${this._stripSecrets(line)}`);
        return;
      }
      this._parentRequests.delete(msg.id);
      if (msg.error) {
        const { message, code, data } = msg.error as RpcError;
        pending.reject(new EngineRequestError(message, { code, data }));
      } else {
        pending.resolve(msg.result);
      }
      return;
    }
    if (msg.method === Method.Delta) {
      deltas.push(JSON.stringify(msg.params));
      return;
    }
    // The one-shot error the engine prints when it can't start (bind failed, no log file).
    if (typeof msg.error === 'string') {
      this._startupError = msg.error;
      return;
    }
    console.warn(`Sync engine wrote an unexpected line: ${this._stripSecrets(line)}`);
  }

  /*
  Joins a `serve --detached` engine found at run/engine.json instead of spawning one: the
  parent methods go over the socket, which a detached engine takes them on, and 'close' comes
  when the connection ends. Shutting down leaves the engine running. Credentials can't be
  imported this way (`credentials.import` is stdin only).
  */
  attach(socketPath: string) {
    this._attached = true;
    this._socketPath = socketPath;
    this._socketQueue = [];
    const finish = (exit: EngineProcessExit) => {
      if (this._closed) return;
      this._closed = true;
      this._closeSocket();
      this.emit('close', exit);
    };
    this.once('hello', () => this.emit('ready'));
    this._connectSocket(socketPath, (error) => {
      if (!this._killed) console.warn(`Detached sync engine connection failed: ${error}`);
      finish({ code: this._killed ? 0 : -2, error: this._killed ? null : error, signal: null });
    });
  }

  _parentCall<R = unknown>(method: string, params: unknown): Promise<R> {
    if (this._attached) {
      if (this._closed) {
        return Promise.reject(
          new EngineRequestError('The sync engine is not connected', { engineExited: true })
        );
      }
      return this.request<R>(method, params);
    }
    const stdin = this._proc && this._proc.stdin;
    if (this._closed || !stdin) {
      return Promise.reject(
        new EngineRequestError('The sync engine is not running', { engineExited: true })
      );
    }
    const id = this._nextParentId++;
    const line = `${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`;
    return new Promise<R>((resolve, reject) => {
      this._parentRequests.set(id, { method, resolve, reject });
      try {
        stdin.write(line, 'utf-8');
      } catch (err) {
        this._parentRequests.delete(id);
        reject(err);
      }
    });
  }

  _rememberSecrets(account: { settings?: Record<string, unknown> }) {
    const settings = (account && account.settings) || {};
    for (const key of ['refresh_token', 'imap_password', 'smtp_password']) {
      const v = settings[key];
      if (typeof v === 'string' && v.length > 0) this._secretValues.add(v);
    }
  }

  // `account` is the account JSON with its secrets; `identity` the identity line, null today.
  addAccount(account: { settings?: Record<string, unknown> }, identity: unknown = null) {
    this._rememberSecrets(account);
    return this._parentCall<AccountAddResult>(Method.AccountAdd, { account, identity });
  }

  // On an engine with Feature.Accounts this also unregisters the account and deletes its
  // stored secrets.
  removeAccount(accountId: string) {
    return this._parentCall<AccountRemoveResult>(Method.AccountRemove, { accountId });
  }

  // Moves accounts and the keyset into the engine (stdin only). Registers without starting.
  importCredentials(params: CredentialsImportParams) {
    for (const account of params.accounts) this._rememberSecrets(account);
    for (const v of Object.values(params.secrets)) {
      if (typeof v === 'string' && v.length > 0) this._secretValues.add(v);
    }
    return this._parentCall<CredentialsImportResult>(Method.CredentialsImport, params);
  }

  // Deletes the account's local data so it syncs from scratch. The engine starts a
  // registered account again itself; the caller's account.add then answers AccountExists. `keepMetadata: false` (removing the account) deletes
  // its plugin metadata too instead of keeping it for the resync.
  resetAccount(accountId: string, { keepMetadata = true }: ResetOptions = {}) {
    return this._parentCall(
      Method.AccountReset,
      keepMetadata ? { accountId } : { accountId, keepMetadata }
    );
  }

  // Removes every account and starts over on an empty database. The caller relaunches.
  // `keepMetadata: false` (the last account was removed) carries no plugin metadata over.
  resetDatabase({ keepMetadata = true }: ResetOptions = {}) {
    return this._parentCall<ResetDatabaseResult>(
      Method.EngineResetDatabase,
      keepMetadata ? {} : { keepMetadata }
    );
  }

  // Verbose logging until `verboseUntil` (ms since the epoch; 0 turns it off). Running
  // accounts restart to pick it up.
  setLogLevel(verboseUntil: number) {
    return this._parentCall<SetLogLevelResult>(Method.EngineSetLogLevel, { verboseUntil });
  }

  // One of the bridge's stdin-shaped commands, for one account.
  sendToAccount(accountId: string, json: { type?: string; [key: string]: any }) {
    console.log(`Sending to the sync engine for ${accountId}`, json);
    const request = requestForCommand(accountId, json);
    if (request) {
      this._sendRequest(request.method, request.params);
    } else if (json.type === 'test-crash') {
      this._parentCall(Method.TestCrash, { accountId }).catch((err) =>
        console.warn(`test.crash failed: ${err}`)
      );
    } else {
      console.warn(`The sync engine has no method for ${json.type}`);
    }
  }

  wakeAll() {
    this._sendRequest(Method.SyncWake, {});
  }

  // Asks the engine to stop every account and exit, and kills it if that takes longer than
  // `graceMs`. Resolves once it has exited.
  shutdown(graceMs = ENGINE_SHUTDOWN_GRACE_MS) {
    if (this._attached) {
      // The detached engine keeps running; only this connection ends.
      if (this._closed) return Promise.resolve();
      this._killed = true;
      const closed = new Promise<void>((resolve) => this.once('close', () => resolve()));
      this._closeSocket();
      this._closed = true;
      this.emit('close', { code: 0, signal: null });
      return closed;
    }
    const proc = this._proc;
    if (this._closed || !proc) return Promise.resolve();
    this._killed = true;
    const closed = new Promise<void>((resolve) => this.once('close', () => resolve()));
    this._parentCall(Method.EngineShutdown, {}).catch(() => {});
    this._shutdownTimer = setTimeout(() => {
      console.warn('The sync engine did not shut down in time; killing it');
      proc.kill();
    }, graceMs);
    return closed;
  }
}
