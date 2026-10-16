import type {
  EngineNotification,
  HelloResult,
  RpcMethodName,
  RpcParamsArg,
  RpcResult,
} from './rpc';
import type { FileJSON } from './types';

export type ConnectionState = 'idle' | 'connecting' | 'open' | 'reconnecting' | 'closed';

export type Unsubscribe = () => void;

export interface EngineClient {
  readonly kind: 'mock' | 'ws';
  readonly state: ConnectionState;

  /** Opens the connection, sends `hello` and `subscribe`. Resolves with the hello result. */
  connect(): Promise<HelloResult>;
  close(): void;

  call<M extends RpcMethodName>(method: M, ...params: RpcParamsArg<M>): Promise<RpcResult<M>>;

  /** Every notification (deltas, account.state) after `subscribe`. */
  onNotification(listener: (n: EngineNotification) => void): Unsubscribe;
  onStateChange(listener: (state: ConnectionState) => void): Unsubscribe;

  // TODO(engine): attachments need an HTTP download route on the hosted engine
  // (e.g. GET /files/:accountId/:fileId) with the same auth as the socket.
  fileUrl(file: FileJSON): string | null;
}
