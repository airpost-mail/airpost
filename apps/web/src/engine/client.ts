import type {
  EngineNotification,
  HelloResult,
  RpcMethodName,
  RpcParamsArg,
  RpcResult,
} from './rpc';
import type { FileJSON } from './types';
import type { ConnectionState, LiveQueries, Unsubscribe } from '@airpost/client';

export { EngineUnavailableError, type ConnectionState, type Unsubscribe } from '@airpost/client';

/**
 * Why the client isn't connected, when it knows: the gateway didn't answer, the gateway
 * answered but the engine isn't running, the engine connection failed or dropped, this
 * browser isn't paired, or the gateway refused the connection for good (a bug or a proxy).
 */
export type ConnectionProblem =
  'server-unreachable' | 'engine-down' | 'engine-unreachable' | 'unpaired' | 'rejected';

export interface EngineClient {
  readonly kind: 'mock' | 'ws';
  readonly state: ConnectionState;
  /** Null while connected, or when the client doesn't know. */
  readonly problem: ConnectionProblem | null;
  /**
   * `hello.features` of the last handshake, kept across reconnects; null before the first.
   * Listeners of `onStateChange` see it set by the time the state is `open`.
   */
  readonly features: ReadonlySet<string> | null;

  /**
   * The connection's live queries (`query.subscribe`); each subscribes again after a
   * reconnect. `engine/live.ts` keeps their rows in the query cache.
   */
  readonly live: LiveQueries;

  /** Opens the connection, sends `hello` and `subscribe`. Resolves with the hello result. */
  connect(): Promise<HelloResult>;
  close(): void;
  /** Skips the rest of a reconnect backoff. */
  retryNow(): void;

  call<M extends RpcMethodName>(method: M, ...params: RpcParamsArg<M>): Promise<RpcResult<M>>;

  /** Every notification (deltas, account.state) after `subscribe`. */
  onNotification(listener: (n: EngineNotification) => void): Unsubscribe;
  /** Also called when `problem` changes. */
  onStateChange(listener: (state: ConnectionState) => void): Unsubscribe;

  /**
   * Where the browser downloads a file: the gateway's file route, or null until the first
   * handshake. `inline` asks for a URL an `<img>` can show (`cid:`
   * parts); the gateway serves only recognised raster images inline.
   */
  fileUrl(file: FileJSON, opts?: { inline?: boolean }): string | null;
  /** Stores a picked file as a new File for a draft of `accountId` (`files.put`). */
  uploadFile(accountId: string, file: Blob, name: string): Promise<FileJSON>;
  /**
   * Makes sure the engine has the file's bytes, downloading the message's body if needed:
   * SendDraftTask attaches files from the engine's files directory by id. Rejects when the
   * file can't be had.
   */
  prepareFile(file: FileJSON): Promise<void>;
}
