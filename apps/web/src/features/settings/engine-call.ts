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
  if (err instanceof RpcError && err.code === RpcErrorCode.NotAllowedRelayed) {
    // The gateway's policy (docs/web-gateway-plan.md G6): account, secret and engine
    // methods need an admin device, and some need DCP stage 12 as well.
    return 'This browser isn’t allowed to do that. Account changes from the web need an admin device and a newer engine.';
  }
  if (err instanceof Error && err.message) return err.message;
  return 'Couldn’t reach the engine.';
}
