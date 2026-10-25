// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

// The shared client for the Tern engine (docs/thin-clients-plan.md section 3.3): the protocol
// types and constants generated from crates/tern-ipc (src/protocol), the JSON-RPC core, the
// session, the WebSocket transport, the settings cache and task waits. No framework and no
// Node: the Unix socket transport is in the `@tern/client/node` entry.
export * from './protocol';
export * from './rpc';
export * from './session';
export * from './settings';
export * from './tasks';
export * from './transports/ws';
