// SPDX-License-Identifier: GPL-3.0-only
// Generated from crates/tern-ipc by packages/client/scripts/generate.mjs. Do not edit: change
// the Rust types, then run `npm run generate` in packages/client.

import type { RpcMethods, RpcNotifications } from './types';

export * from './types';
export * from './constants';

export type RpcMethodName = keyof RpcMethods;
export type RpcParams<M extends RpcMethodName> = RpcMethods[M]['params'];
export type RpcResult<M extends RpcMethodName> = RpcMethods[M]['result'];
/** Lets `call('engine.status')` leave out params while every other method takes them. */
export type RpcParamsArg<M extends RpcMethodName> =
  RpcParams<M> extends undefined ? [params?: undefined] : [params: RpcParams<M>];
export type RpcNotificationName = keyof RpcNotifications;
/** A notification as a tagged union, for a `switch (n.method)`. */
export type EngineNotification = {
  [K in RpcNotificationName]: { method: K; params: RpcNotifications[K] };
}[RpcNotificationName];
