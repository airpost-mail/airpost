import type {
  EngineNotification,
  HelloResult,
  RpcMethodName,
  RpcParamsArg,
  RpcResult,
} from './rpc';
import type { FileJSON } from './types';

export type ConnectionState = 'idle' | 'connecting' | 'open' | 'reconnecting' | 'closed';

/**
 * Why the client isn't connected, when it knows: the gateway didn't answer, the gateway
 * answered but the engine isn't running, the engine connection failed or dropped, this
 * browser isn't paired, or the gateway refused the connection for good (a bug or a proxy).
 */
export type ConnectionProblem =
  'server-unreachable' | 'engine-down' | 'engine-unreachable' | 'unpaired' | 'rejected';

export type Unsubscribe = () => void;

/** A call that couldn't be sent: no connection within the wait, or the connection dropped. */
export class EngineUnavailableError extends Error {
  constructor(message = 'The engine is not connected.') {
    super(message);
    this.name = 'EngineUnavailable';
  }
}

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

  // TODO(engine): downloads need the gateway's file route, `GET /gateway/v1/files/:accountId/
  // :fileId` on DCP stage 16's `files.get` (docs/web-gateway-plan.md 2.5, step W5).
  fileUrl(file: FileJSON): string | null;
}
