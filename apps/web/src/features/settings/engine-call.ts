import { ensureConnected, RpcError, RpcErrorCode, type EngineClient } from '@/engine';
import type { RpcMethodName, RpcParamsArg, RpcResult } from '@/engine';

/** `client.call` after the connection is up. */
export async function callEngine<M extends RpcMethodName>(
  client: EngineClient,
  method: M,
  ...params: RpcParamsArg<M>
): Promise<RpcResult<M>> {
  await ensureConnected(client);
  return client.call(method, ...params);
}

/** A short, plain message for an engine error. */
export function engineErrorMessage(err: unknown): string {
  if (err instanceof RpcError && err.code === RpcErrorCode.MethodNotFound) {
    return 'The engine doesn’t support this yet.';
  }
  if (err instanceof Error && err.message) return err.message;
  return 'Couldn’t reach the engine.';
}
