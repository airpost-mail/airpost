/* eslint global-require: 0 */

/*
Warning! This file is imported from the main process as well as the renderer process
*/
import { spawn, exec, ChildProcess } from 'child_process';
import { Readable } from 'stream';
import { StringDecoder } from 'string_decoder';
import path from 'path';
import os from 'os';
import { EventEmitter } from 'events';
import fs from 'fs';
import net from 'net';
import { localized } from './intl';
import { developerToolsAvailable } from './developer-tools';
import { Account } from 'mailspring-exports';
import {
  AccountAddResult,
  AccountRemoveResult,
  AccountStateParams,
  DisconnectParams,
  engineSocketPath,
  HelloParams,
  HelloResult,
  MailNewParams,
  MAX_LINE_BYTES,
  Method,
  parseListeningLine,
  PROTOCOL_VERSION,
  RequestId,
  requestForCommand,
  RpcResponse,
  RpcError,
  serveSocketPath,
  SubscribeParams,
  deltaParamsText,
} from './engine-protocol';

import {
  GMAIL_CLIENT_ID,
  GMAIL_CLIENT_SECRET,
} from '../internal_packages/onboarding/lib/onboarding-constants';

let Utils = null;

export interface MailsyncProcessExit {
  code: number;
  error?: Error;
  signal: string;
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

// LocalizedErrorStrings codes that, despite being "classified" by mailsync,
// can also indicate a genuine Mailspring defect rather than a server/user
// condition - a malformed-response parser regression. These should keep
// reaching error reporting instead of being treated as expected,
// user-actionable failures.
const AMBIGUOUS_MAILSYNC_ERRORS = new Set(['ErrorParse']);

/*
Extracts mailsync's JSON result from what it wrote to stdout.

The result is one JSON object, but the end of the stream is not reliably that object. It
arrives without a trailing newline, mailsync prints human-readable progress lines before it
(`Running Setup`), and on Linux it can emit a diagnostic - a missing libtidy is the common
one - at any point. Parsing the last line of stdout and stderr combined turns any of those
into "an unknown error has occurred mailsync: 0", so scan stdout backwards for the last
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

/*
Which sync engine to run: the C++ `mailsync` or the Rust `mailsync-rs`, shipped side by side
in app.asar.unpacked. `core.sync.engine` in config.json picks one, `MAILSYNC_ENGINE` overrides
it, and C++ stays the default until the Rust engine has been dogfooded.

The main process resolves this once at startup, before `migrate`, and hands the result to
every window in its load settings, so all processes of one launch run the engine that
migrated the database. A changed setting takes effect on the next launch.
*/
export type SyncEngine = 'cpp' | 'rust';

const SYNC_ENGINE_BINARIES: { [engine in SyncEngine]: string } = {
  cpp: 'mailsync',
  rust: 'mailsync-rs',
};

function syncEngineBinaryPath(resourcePath: string, engine: SyncEngine) {
  return path
    .join(resourcePath, SYNC_ENGINE_BINARIES[engine])
    .replace('app.asar', 'app.asar.unpacked');
}

function configuredSyncEngine(configDirPath: string): SyncEngine {
  const fromEnv = process.env.MAILSYNC_ENGINE;
  if (fromEnv === 'cpp' || fromEnv === 'rust') {
    return fromEnv;
  }
  // Read config.json directly, as intl.ts does: this runs before Config is loaded.
  try {
    const config = JSON.parse(fs.readFileSync(path.join(configDirPath, 'config.json')).toString());
    if (config['*'].core.sync.engine === 'rust') {
      return 'rust';
    }
  } catch (err) {
    // first run, or no engine setting
  }
  return 'cpp';
}

// A build without mailsync-rs (no packaging step put it there, or Windows) runs the C++
// engine rather than failing to launch on a setting the user can only undo in config.json.
export function resolveSyncEngine(configDirPath: string, resourcePath: string): SyncEngine {
  const engine = configuredSyncEngine(configDirPath);
  if (engine === 'rust' && !fs.existsSync(syncEngineBinaryPath(resourcePath, 'rust'))) {
    console.warn(
      `The Rust sync engine is selected but ${SYNC_ENGINE_BINARIES.rust} is not in this ` +
        `build; running the C++ engine`
    );
    return 'cpp';
  }
  // Source checkouts of the monorepo carry no C++ binary, only the Rust one the engine build
  // installs.
  if (
    engine === 'cpp' &&
    !fs.existsSync(syncEngineBinaryPath(resourcePath, 'cpp')) &&
    fs.existsSync(syncEngineBinaryPath(resourcePath, 'rust'))
  ) {
    console.warn(
      `The C++ sync engine is selected but ${SYNC_ENGINE_BINARIES.cpp} is not in this ` +
        `build; running the Rust engine`
    );
    return 'rust';
  }
  return engine;
}

/*
The Rust engine's `migrate` moves a C++ database aside itself. The C++ engine opens a
`user_version = 12` database without migrating it (MailStore.cpp), so on a database the Rust
engine wrote it would run on Rust-generated ids. Before C++ migrates, move such a database to
edgehill.rust.db* so C++ starts from an empty one and resyncs.
*/
function moveAsideRustDatabase(configDirPath: string) {
  const dbPath = path.join(configDirPath, 'edgehill.db');
  if (!fs.existsSync(dbPath)) return;

  let engine = null;
  let localOnly = null;
  try {
    const Sqlite3 = require('better-sqlite3');
    const db = new Sqlite3(dbPath, { fileMustExist: true });
    try {
      const row = db.prepare('SELECT value FROM `_State` WHERE id = ?').get('engine');
      engine = row ? row.value : null;
      if (engine === 'rust') {
        localOnly = countLocalOnlyRows(db);
      }
    } finally {
      db.close();
    }
  } catch (err) {
    // Unreadable or not one of ours; let mailsync's own migration decide.
    return;
  }
  if (engine !== 'rust') return;

  console.log('Moving the Rust engine database aside before the C++ engine migrates');
  // Nothing imports from edgehill.rust.db, and the next switch back replaces it.
  if (localOnly && (localOnly.drafts > 0 || localOnly.scheduled > 0)) {
    console.warn(
      `Switching to the C++ engine resyncs from the server. ${localOnly.drafts} unsent local ` +
        `draft(s) and ${localOnly.scheduled} pending snooze/send-later/reminder value(s) stay ` +
        `only in edgehill.rust.db, which the next switch from Rust to C++ replaces.`
    );
  }
  for (const suffix of ['', '-wal', '-shm']) {
    const from = `${dbPath}${suffix}`;
    const to = path.join(configDirPath, `edgehill.rust.db${suffix}`);
    if (fs.existsSync(to)) fs.unlinkSync(to);
    if (fs.existsSync(from)) fs.renameSync(from, to);
  }
}

/* Drafts never uploaded (a UID-0 placement only) and metadata waiting on an expiration:
what a resync from the server does not bring back to an account without a metadata identity. */
function countLocalOnlyRows(db: any): { drafts: number; scheduled: number } | null {
  try {
    const drafts = db
      .prepare(
        'SELECT count(*) AS n FROM Message m WHERE m.draft = 1 AND EXISTS ' +
          '(SELECT 1 FROM MessageFolder p WHERE p.messageId = m.id AND p.remoteUID = 0) ' +
          'AND NOT EXISTS ' +
          '(SELECT 1 FROM MessageFolder p WHERE p.messageId = m.id AND p.remoteUID > 0)'
      )
      .get().n;
    const scheduled = db
      .prepare('SELECT count(*) AS n FROM ModelPluginMetadata WHERE expiration > ?')
      .get(Math.floor(Date.now() / 1000)).n;
    return { drafts, scheduled };
  } catch (err) {
    return null;
  }
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

export class MailsyncProcess extends EventEmitter {
  _proc: ChildProcess = null;
  _win = null;

  // Socket transport (Rust engine, sync mode). `_socketQueue` holds requests sent before the
  // engine printed its listening line; it is null once connected and after the connection ends.
  _socketPath: string = null;
  _socket: net.Socket = null;
  _socketQueue: string[] = null;
  _socketRequests = new Map<RequestId, string>();
  // Socket requests whose answer someone awaits (request()), by id.
  _socketCalls = new Map<
    RequestId,
    { resolve: (result: any) => void; reject: (error: Error) => void }
  >();
  // `hello.features` of the engine on the socket; null until hello answers.
  features: string[] = null;
  _nextRequestId = 1;
  _killed = false;

  // these must be set before you use the process
  account: Account = null;

  verbose: boolean;
  resourcePath: string;
  configDirPath: string;
  syncEngine: SyncEngine;
  binaryPath: string;

  constructor({
    configDirPath,
    resourcePath,
    verbose,
    syncEngine,
  }: {
    configDirPath: string;
    resourcePath: string;
    verbose?: boolean;
    syncEngine?: SyncEngine;
  }) {
    super();
    this.verbose = verbose;
    this.resourcePath = resourcePath;
    this.configDirPath = configDirPath;
    this.syncEngine = syncEngine || resolveSyncEngine(configDirPath, resourcePath);
    this.binaryPath = syncEngineBinaryPath(resourcePath, this.syncEngine);
  }

  _showStatusWindow(mode) {
    if (this._win) return;

    let BrowserWindow;
    if (process.type === 'renderer') {
      BrowserWindow = require('@electron/remote').BrowserWindow;
    } else {
      BrowserWindow = require('electron').BrowserWindow;
    }

    this._win = new BrowserWindow({
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
    this._win.setContentSize(350, 90);
    this._win.once('ready-to-show', () => {
      this._win.show();
    });
    this._win.loadURL(`file://${this.resourcePath}/static/db-${mode}.html`);
  }

  _closeStatusWindow() {
    if (!this._win) return;
    this._win.removeAllListeners('ready-to-show');
    this._win.setClosable(true);
    this._win.hide();
    setTimeout(() => {
      // don't know why this timeout is necessary but the app becomes unable to
      // load Electron modules in the main process if we close immediately.
      if (!this._win.isDestroyed()) this._win.close();
      this._win = null;
    });
  }

  _spawnProcess(mode, extraArgs: string[] = []) {
    // Build on a null-prototype object so the spawned engine inherits only
    // genuine environment variables, not properties a prototype-pollution bug
    // elsewhere in the process could have added to Object.prototype. See
    // GHSA-gjr7-3mj2-cr54.
    const env = Object.assign(Object.create(null), process.env, {
      CONFIG_DIR_PATH: this.configDirPath,
      GMAIL_CLIENT_ID: GMAIL_CLIENT_ID,
      GMAIL_CLIENT_SECRET: GMAIL_CLIENT_SECRET,
    });
    // The C++ engine prints its usage and exits when this is empty. Without an identity it
    // only reads it in install-check mode, which the app doesn't run.
    if (this.syncEngine === 'cpp') {
      env.IDENTITY_SERVER = 'unknown';
    }

    const args = [`--mode`, mode];
    if (this.verbose) {
      args.push('--verbose');
    }
    if (this.account) {
      args.push('--info', this.account.emailAddress);
    }
    args.push(...extraArgs);
    this._proc = spawn(this.binaryPath, args, { env });

    /* Allow us to buffer up to 1MB on stdin instead of 16k. This is necessary
    because some tasks (creating replies to drafts, etc.) can be gigantic amounts
    of HTML, many tasks can be created at once, etc, and we don't want to kill
    the channel. */
    if (this._proc.stdin) {
      this._proc.stdin.setDefaultEncoding('utf-8');
      (this._proc.stdin as any).highWaterMark = 1024 * 1024;
    }

    // stdout may not be present if an error occurred. Error handler hasn't been
    // attached yet, but will be by the caller of spawnProcess.
    if (this.account && this._proc.stdout) {
      this._proc.stdout.once('data', () => {
        const rs = new Readable();
        // Both engines read an identity line after the account; `null` means none.
        rs.push(`${JSON.stringify(this.account)}\nnull\n`);
        rs.push(null);
        rs.pipe(this._proc.stdin, { end: false });
      });
    }
  }

  // Redacts known secrets from a log/error string before it's surfaced to
  // the UI, error reporting, or thrown errors. Two strategies are combined:
  //
  // 1. Key-based: replace the value of any known sensitive JSON key (e.g.
  //    "refresh_token":"abc"). This catches secrets the engine has rotated
  //    and not yet pushed back to us (e.g. ProcessAccountSecretsUpdated),
  //    where the new value is not present in this.account.
  // 2. Value-based: replace literal occurrences of the secrets currently
  //    cached on this.account, in case they appear outside JSON form.
  _stripSecrets(text: string) {
    if (!text) return text;
    let out = text;

    const SENSITIVE_JSON_KEYS = ['refresh_token', 'access_token', 'imap_password', 'smtp_password'];
    for (const key of SENSITIVE_JSON_KEYS) {
      // Match "key":"<anything-but-unescaped-quote>" allowing escaped quotes inside.
      const re = new RegExp(`("${key}"\\s*:\\s*")(?:\\\\.|[^"\\\\])+(")`, 'g');
      out = out.replace(re, '$1*********$2');
    }

    const cachedSettings = (this.account && this.account.settings) || ({} as any);
    const cachedValues = [
      cachedSettings.refresh_token,
      cachedSettings.imap_password,
      cachedSettings.smtp_password,
    ].filter((v): v is string => typeof v === 'string' && v.length > 0);

    for (const v of cachedValues) {
      out = out.replaceAll(v, '*********');
    }

    return out;
  }

  _spawnAndWait(mode, { onData }: { onData?: (data: any) => void } = {}) {
    return new Promise<{ response: any; buffer: Buffer }>((resolve, reject) => {
      this._spawnProcess(mode);
      // `bothChunks` is both streams together and is what we show the user when something
      // goes wrong; `outChunks` is stdout alone, which is the only place the JSON result
      // appears. Chunks are concatenated as bytes and decoded once at the end: `a += b` on
      // two Buffers coerces both through toString(), which decodes every chunk on its own
      // and mangles any multi-byte character that straddles a chunk boundary.
      const bothChunks: Buffer[] = [];
      const outChunks: Buffer[] = [];

      if (this._proc.stdout) {
        this._proc.stdout.on('data', (data: Buffer) => {
          bothChunks.push(data);
          outChunks.push(data);
          if (onData) onData(data);
        });
      }
      if (this._proc.stderr) {
        this._proc.stderr.on('data', (data: Buffer) => {
          bothChunks.push(data);
          if (onData) onData(data);
        });
      }

      this._proc.on('error', (err: Error) => {
        reject(err);
      });

      this._proc.on('close', (code, signal) => {
        const buffer = Buffer.concat(bothChunks);
        try {
          const response = lastJSONResponse(Buffer.concat(outChunks).toString('utf-8'));
          if (!response) {
            // If the Mailsync executable itself failed to run, the logs are not JSON
            // and may contain system errors (shared library issues, etc). Include this
            // in the logs so users can fix on their own or report detailed bugs.
            const rawLog = this._stripSecrets(buffer.toString());
            return reject(this._buildCrashError(code, signal, rawLog));
          }

          if (code === 0) {
            resolve({ response, buffer });
          } else {
            // Mailsync executed fine, and this is an mailsync error in JSON format
            const isRecognizedMailsyncError = Object.prototype.hasOwnProperty.call(
              LocalizedErrorStrings,
              response.error
            );
            let msg = isRecognizedMailsyncError
              ? LocalizedErrorStrings[response.error]
              : response.error;
            if (response.error_service) {
              msg = `${msg} (${response.error_service.toUpperCase()})`;
            }
            // Set only for a rejected TLS handshake today; arrives unlocalized.
            if (response.error_advice) {
              msg = `${msg} ${response.error_advice}`;
            }
            const error = new Error(msg);
            (error as any).rawLog = this._stripSecrets(response.log);
            (error as any).errorAdvice = response.error_advice || null;
            (error as any).errorService = response.error_service || null;
            // Set when the engine classified the failure as this machine being unable to
            // reach the network, rather than a server refusing or rejecting us.
            (error as any).isNetworkError = response.error_offline === true;
            // Errors mailsync explicitly classified (bad credentials, unreachable
            // server, TLS/certificate problems, provider-side rate limits, etc.)
            // describe the mail server or the user's settings, not a bug in
            // Mailspring - except for the ambiguous codes above, which we still
            // want reported if they occur. Callers that report unexpected errors
            // to Sentry (e.g. oauth-signin-page's error handler) check this flag
            // to avoid flooding error tracking with expected, user-actionable
            // failures.
            (error as any).isUserError =
              isRecognizedMailsyncError && !AMBIGUOUS_MAILSYNC_ERRORS.has(response.error);
            return reject(error);
          }
        } catch (err) {
          const rawLog = this._stripSecrets(buffer.toString());
          return reject(this._buildCrashError(code, signal, rawLog));
        }
      });
    });
  }

  // Called when the mailsync child process wrote no parseable JSON result, meaning it
  // crashed outright or never ran: a segfault inside mailcore, the executable-path check
  // that exits 2, a missing shared library. The raw log is part of the message because
  // those are diagnosable by the user or by whoever reads the bug report. A process
  // terminated by a signal has a null `code`, which would otherwise read "mailsync: null".
  _buildCrashError(code: number | null, signal: NodeJS.Signals | null, rawLog: string) {
    const exitDescription = signal ? `signal ${signal}` : `${code}`;
    const error = new Error(
      `${localized(`An unknown error has occurred`)} mailsync: ${exitDescription}. ${rawLog}`
    );
    (error as any).rawLog = rawLog;
    return error;
  }

  kill() {
    console.warn('Terminating mailsync...');
    this._killed = true;
    this._proc && this._proc.kill();
  }

  /*
  The socket the Rust engine should serve its protocol on, or null for the stdio transport:
  the C++ engine, Windows, TERN_ENGINE_TRANSPORT=stdio (for debugging), or no usable path.
  */
  _socketTransportPath(): string | null {
    if (this.syncEngine !== 'rust' || process.platform === 'win32' || !this.account) {
      return null;
    }
    if (process.env.TERN_ENGINE_TRANSPORT === 'stdio') {
      return null;
    }
    const location = engineSocketPath(this.configDirPath, this.account.id);
    if (!location) {
      console.warn('No socket path short enough for the sync engine; using stdio');
      return null;
    }
    if (location.ownDir) {
      try {
        ensurePrivateDir(location.ownDir);
      } catch (err) {
        console.warn(
          `Cannot use ${location.ownDir} for the sync engine socket (${err}); using stdio`
        );
        return null;
      }
    }
    return location.socketPath;
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
  Connects to the engine once it printed its listening line, says hello, subscribes to every
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
    this._sendRequest(Method.Subscribe, { accounts: null, classes: null } as SubscribeParams);
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
      inBuffer = lines.pop();
      const deltas: string[] = [];
      let failure: Error = null;
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
    if (msg.method === Method.Disconnect) {
      const { reason } = (msg.params || {}) as DisconnectParams;
      console.warn(`Sync engine dropped the connection: ${reason}`);
      return new Error(`The sync engine dropped the connection: ${reason}`);
    }
    if (msg.method === undefined && msg.id !== undefined) {
      const response = msg as RpcResponse;
      const method = this._socketRequests.get(response.id);
      this._socketRequests.delete(response.id);
      const call = this._socketCalls.get(response.id);
      this._socketCalls.delete(response.id);
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

  sync() {
    const socketPath = this._socketTransportPath();
    this._socketPath = socketPath;
    this._socketQueue = socketPath ? [] : null;
    this._spawnProcess('sync', socketPath ? ['--socket', socketPath] : []);
    let outBuffer = '';
    let errBuffer = '';
    let cleanedUp = false;
    let socketCloseTimer = null;

    // Like a stdin EPIPE: kill the engine and report code -2 so the bridge relaunches it.
    const failSocket = (error: Error, { now }: { now?: boolean } = {}) => {
      if (cleanedUp || this._killed) return;
      const finish = () => {
        if (cleanedUp || this._killed) return;
        cleanedUp = true;
        console.warn(`Sync engine socket failed: ${error}`);
        this._closeSocket();
        this._proc.kill();
        this.emit('close', { code: -2, error, signal: null });
      };
      if (now) {
        finish();
      } else if (!socketCloseTimer) {
        socketCloseTimer = setTimeout(finish, SOCKET_CLOSE_GRACE_MS);
      }
    };

    if (this._proc.stdout) {
      this._proc.stdout.on('data', (data) => {
        const added = data.toString();
        try {
          outBuffer += added;
        } catch (err) {
          console.error(`Mailsync process buffer is ${outBuffer.length} chars, out of memory.`);
          outBuffer = '';
        }

        if (added.indexOf('\n') !== -1) {
          let msgs = outBuffer.split('\n');
          outBuffer = msgs.pop();
          if (socketPath) {
            // stdout still carries the handshake prompts and ProcessAccountSecretsUpdated.
            msgs = msgs.filter((msg) => {
              const listening = parseListeningLine(msg);
              if (listening === null) return true;
              this._connectSocket(listening, failSocket);
              return false;
            });
          }
          this.emit('deltas', msgs);
        }
      });
    }
    if (this._proc.stderr) {
      this._proc.stderr.on('data', (data) => {
        try {
          errBuffer += data.toString();
          // Trim to last 100KB if the buffer grows too large to avoid OOM
          if (errBuffer.length > 100 * 1024) {
            errBuffer = errBuffer.slice(-100 * 1024);
          }
        } catch (err) {
          console.error(`Mailsync stderr buffer is ${errBuffer.length} chars, out of memory.`);
          errBuffer = '';
        }
      });
    }
    // Note: we intentionally do not re-emit this as an 'error' event on `this`.
    // Nothing in the codebase attaches an 'error' listener to a MailsyncProcess
    // instance, and Node's EventEmitter throws synchronously when an 'error'
    // event has no listeners. That throw happened inside this same 'error'
    // callback on `_proc`, which aborted the remaining `_proc.on('error', ...)`
    // listener below (the one that actually cleans up and reports failure via
    // 'close') before it could run — so a transient spawn failure (e.g. EIO)
    // both crashed the app and prevented MailsyncBridge from ever marking the
    // account as errored or retrying.
    this._proc.on('error', (err: Error) => {
      console.log(`Sync worker exited with ${err}`);
    });

    // Handle EPIPE and other stdin errors that occur when the child process
    // exits while we're still trying to write to it. These errors are emitted
    // asynchronously as 'error' events on stdin rather than thrown from write(),
    // so the try/catch in sendMessage() does not catch them.
    if (this._proc.stdin) {
      this._proc.stdin.on('error', (err: Error) => {
        if (cleanedUp) return;
        cleanedUp = true;
        this._closeSocket();
        this._proc.kill();
        this.emit('close', { code: -2, error: err, signal: null });
      });
    }

    const onStreamCloseOrExit = (code: number, signal: string) => {
      if (socketCloseTimer) {
        clearTimeout(socketCloseTimer);
        socketCloseTimer = null;
      }
      this._closeSocket();
      if (cleanedUp) {
        return;
      }

      let error = null;
      let lastJSON = null;
      try {
        if (outBuffer.length) {
          // Skip debug output that starts with 'dbg::' prefix
          if (outBuffer.startsWith('dbg::')) {
            console.log('Skipping debug output from mailsync:', this._stripSecrets(outBuffer));
          } else {
            lastJSON = JSON.parse(outBuffer);
          }
        }
      } catch (parseError) {
        console.warn('Failed to parse mailsync output as JSON:', this._stripSecrets(outBuffer));
      } finally {
        if (lastJSON) {
          if (lastJSON.error) {
            error = new Error(this._stripSecrets(lastJSON.error));
          } else {
            this.emit('deltas', [outBuffer]);
          }
        }
      }

      if (errBuffer) {
        error = new Error(this._stripSecrets(errBuffer));
      }

      cleanedUp = true;
      this.emit('close', { code, error, signal } as MailsyncProcessExit);
    };

    this._proc.on('error', (error) => {
      if (cleanedUp) {
        return;
      }
      cleanedUp = true;
      this.emit('close', { code: -1, error, signal: null });
    });
    this._proc.on('close', onStreamCloseOrExit);
    this._proc.on('exit', onStreamCloseOrExit);
  }

  sendMessage(json) {
    if (!Utils) {
      Utils = require('mailspring-exports').Utils;
    }
    console.log(`Sending to mailsync ${this.account ? this.account.id : '?'}`, json);
    const request = this._socketPath ? requestForCommand(this.account.id, json) : null;
    if (request) {
      this._sendRequest(request.method, request.params);
      return;
    }
    const msg = `${JSON.stringify(json)}\n`;
    try {
      this._proc.stdin.write(msg, 'utf-8');
    } catch (error) {
      if (error && (error.message.includes('socket has been ended') || error.code === 'EPIPE')) {
        // The process probably already exited and we missed it somehow,
        // but try to kill it anyway and then force-emit a 'close' to trigger
        // the bridge to restart us.
        this._proc.kill();
        this.emit('close', { code: -2, error, signal: null });
      }
    }
  }

  async migrate() {
    if (this.syncEngine === 'cpp') {
      moveAsideRustDatabase(this.configDirPath);
    }
    try {
      console.log(`Running database migrations (${this.syncEngine} engine)`);
      const { buffer } = await this._spawnAndWait('migrate', {
        onData: (data) => {
          const str = data.toString().toLowerCase();
          if (str.includes('running migration')) this._showStatusWindow('migration');
          if (str.includes('running vacuum')) this._showStatusWindow('vacuum');
        },
      });
      console.log(this._stripSecrets(buffer.toString()));
      // Import while the status window is still up: closing it must be the last step before
      // the main window opens, or the main process stalls with no window (see
      // _closeStatusWindow).
      if (this.syncEngine === 'rust') {
        await this._importLegacy();
      }
      this._closeStatusWindow();
    } catch (err) {
      this._closeStatusWindow();
      throw err;
    }
  }

  // Carries local drafts, snoozes, reminders and send-later over from the C++ database that
  // the Rust engine's migrate moved aside (docs/engine-port-plan.md section 9). A no-op
  // without one or once imported, so it runs on every launch; a failure is logged and the
  // launch continues, since the account processes do not depend on it.
  async _importLegacy() {
    try {
      const { buffer } = await this._spawnAndWait('import-legacy');
      console.log(this._stripSecrets(buffer.toString()));
    } catch (err) {
      console.error(`mailsync --mode import-legacy failed: ${err}`);
    }
  }

  resetCache() {
    return this._spawnAndWait('reset');
  }

  test() {
    return this._spawnAndWait('test');
  }

  attachToXcode() {
    const tmppath = path.join(os.tmpdir(), 'attach.applescript');
    fs.writeFileSync(
      tmppath,
      `
tell application "Xcode"
  activate
end tell

tell application "System Events"
  tell application process "Xcode"
    click (menu item "Attach to Process by PID or Name…" of menu 1 of menu bar item "Debug" of menu bar 1)
  end tell
  tell application process "Xcode"
    set value of text field 1 of sheet 1 of window 1 to "${this._proc.pid}"
  end tell
  delay 0.5
  tell application process "Xcode"
    click button "Attach" of sheet 1 of window 1
  end tell
  
end tell
    `
    );
    exec(`osascript ${tmppath}`);
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
    { code = null, data, engineExited = false }: Partial<RpcError> & { engineExited?: boolean } = {}
  ) {
    super(message);
    this.code = code;
    this.data = data;
    this.engineExited = engineExited;
  }
}

/*
The Rust engine's `serve` mode: one process for every account (docs/engine-protocol-plan.md
section 4). stdin and stdout are the parent channel: JSON-RPC requests for account.add,
account.remove and engine.shutdown, their responses, and the accounts'
ProcessAccountSecretsUpdated deltas, so credentials never touch the socket. Everything else
goes over the socket, connected once the first stdin response shows the engine has bound it.

Emits 'ready' once the engine has bound its socket, 'hello' (HelloResult) once the socket is
greeted, 'deltas' (delta params as JSON text, the shape the bridge reads), 'account-state'
(AccountStateParams), 'mail-new' (MailNewParams), and 'close' (MailsyncProcessExit) once, when
the engine exits or its socket fails.
*/
export class EngineServeProcess extends MailsyncProcess {
  _parentRequests = new Map<
    number,
    { method: string; resolve: (result: any) => void; reject: (error: Error) => void }
  >();
  _nextParentId = 1;
  _closed = false;
  _shutdownTimer = null;
  _startupError: string = null;
  // The secrets of every account added, redacted from what the engine reports. Kept after a
  // remove, since a late error may still carry them.
  _secretValues = new Set<string>();

  serve() {
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

    const decoder = new StringDecoder('utf8');
    let outBuffer = '';
    let errBuffer = '';
    let socketCloseTimer = null;

    const finish = (exit: MailsyncProcessExit) => {
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

    // As in sync(): a dropped socket gets a moment for the real exit to arrive, then the
    // engine is killed and the close reports -2.
    const failSocket = (error: Error, { now }: { now?: boolean } = {}) => {
      if (this._closed || this._killed) return;
      const kill = () => {
        if (this._closed || this._killed) return;
        console.warn(`Sync engine socket failed: ${error}`);
        this._proc.kill();
        finish({ code: -2, error, signal: null });
      };
      if (now) {
        kill();
      } else if (!socketCloseTimer) {
        socketCloseTimer = setTimeout(kill, SOCKET_CLOSE_GRACE_MS);
      }
    };

    if (this._proc.stdout) {
      this._proc.stdout.on('data', (data: Buffer) => {
        outBuffer += decoder.write(data);
        if (outBuffer.indexOf('\n') === -1) return;
        const lines = outBuffer.split('\n');
        outBuffer = lines.pop();
        const deltas: string[] = [];
        for (const line of lines) {
          this._onParentLine(line, deltas);
        }
        if (deltas.length) {
          this.emit('deltas', deltas);
        }
      });
    }
    if (this._proc.stderr) {
      this._proc.stderr.on('data', (data: Buffer) => {
        errBuffer += data.toString();
        if (errBuffer.length > 100 * 1024) {
          errBuffer = errBuffer.slice(-100 * 1024);
        }
      });
    }
    // See sync(): never re-emitted as 'error' on `this`.
    this._proc.on('error', (error: Error) => {
      console.log(`Sync engine exited with ${error}`);
      finish({ code: -1, error, signal: null });
    });
    if (this._proc.stdin) {
      this._proc.stdin.on('error', (error: Error) => {
        if (this._closed) return;
        this._proc.kill();
        finish({ code: -2, error, signal: null });
      });
    }
    const onExit = (code: number, signal: string) => {
      const text = errBuffer || this._startupError;
      finish({ code, error: text ? new Error(this._stripSecrets(text)) : null, signal });
    };
    this._proc.on('close', onExit);
    this._proc.on('exit', onExit);

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

  _parentCall<R = unknown>(method: string, params: unknown): Promise<R> {
    if (this._closed || !this._proc || !this._proc.stdin) {
      return Promise.reject(
        new EngineRequestError('The sync engine is not running', { engineExited: true })
      );
    }
    const id = this._nextParentId++;
    const line = `${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`;
    return new Promise<R>((resolve, reject) => {
      this._parentRequests.set(id, { method, resolve, reject });
      try {
        this._proc.stdin.write(line, 'utf-8');
      } catch (err) {
        this._parentRequests.delete(id);
        reject(err);
      }
    });
  }

  // `account` is the account JSON with its secrets; `identity` the identity line, null today.
  addAccount(account: { settings?: Record<string, unknown> }, identity: unknown = null) {
    const settings = (account && account.settings) || {};
    for (const key of ['refresh_token', 'imap_password', 'smtp_password']) {
      const v = settings[key];
      if (typeof v === 'string' && v.length > 0) this._secretValues.add(v);
    }
    return this._parentCall<AccountAddResult>(Method.AccountAdd, { account, identity });
  }

  removeAccount(accountId: string) {
    return this._parentCall<AccountRemoveResult>(Method.AccountRemove, { accountId });
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
    if (this._closed || !this._proc) return Promise.resolve();
    this._killed = true;
    const closed = new Promise<void>((resolve) => this.once('close', () => resolve()));
    this._parentCall(Method.EngineShutdown, {}).catch(() => {});
    this._shutdownTimer = setTimeout(() => {
      console.warn('The sync engine did not shut down in time; killing it');
      this._proc.kill();
    }, graceMs);
    return closed;
  }
}
