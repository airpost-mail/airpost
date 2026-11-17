// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

/**
 * The shared client for the Airpost engine: the protocol types and constants generated from
 * crates/airpost-ipc (src/protocol), the JSON-RPC core, the session, live queries, the
 * WebSocket transport, the settings cache and task waits. No framework and no Node: the Unix
 * socket transport is in the `@airpost/client/node` entry.
 */
export * from './protocol';
export * from './rpc';
export * from './live';
export * from './session';
export * from './settings';
export * from './tasks';
export * from './transports/ws';
