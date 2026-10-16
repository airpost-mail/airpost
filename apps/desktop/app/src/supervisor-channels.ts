// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
IPC between the main process, which owns the `serve` engine and the keyset
(browser/engine-supervisor.ts, browser/key-manager.ts), and the windows. Imported from both
sides, so it holds only names and shapes.
*/

// ipcRenderer.invoke(KEY_MANAGER_CHANNEL, method, ...args) -> KeyManagerReply
export const KEY_MANAGER_CHANNEL = 'key-manager:call';

export type KeyManagerMethod =
  | 'getKeyHash'
  | 'accountSecrets'
  | 'storeAccountSecrets'
  | 'deleteAccountSecrets'
  | 'getPassword'
  | 'replacePassword'
  | 'deletePassword';

// Errors cross IPC as text; `noSentry` is set when main already showed the user a dialog.
export interface KeyManagerReply {
  value?: any;
  error?: string;
  noSentry?: boolean;
}

export const SupervisorChannel = {
  // invoke -> EngineStatus. Every window asks once at boot, then follows Status.
  GetStatus: 'engine-supervisor:get-status',
  // main -> every window, whenever the engine or the state of an account changes.
  Status: 'engine-supervisor:status',
  // invoke from the main window -> AccountEvent[]: the events it missed while it had no
  // renderer (boot, reload). From then on events arrive as AccountEvent messages.
  AttachMainWindow: 'engine-supervisor:attach-main-window',
  // main -> the main window, with a `seq`; the main window sends AckAccountEvents with the
  // last one it applied.
  AccountEvent: 'engine-supervisor:account-event',
  AckAccountEvents: 'engine-supervisor:ack-account-events',
  // invoke {accountId}
  ForceRelaunch: 'engine-supervisor:force-relaunch',
  // invoke {accountId} -> {error?: string}
  ResetCache: 'engine-supervisor:reset-cache',
  // invoke {account} (with secrets) -> AccountTestReply. Onboarding's settings check.
  TestAccount: 'engine-supervisor:test-account',
  // invoke -> {error?: string}. Removes every account and empties the database; the caller
  // relaunches.
  ResetDatabase: 'engine-supervisor:reset-database',
} as const;

// `account.test`'s result (a failed test is a result with `error`), or why the engine could not
// run it.
export interface AccountTestReply {
  result?: { account: Record<string, any> | null; error: string | null; [key: string]: any };
  error?: string;
}

// What the main window's connection needs to route a command: EngineClient's entry state
// ('adding' until account.add answered, 'removing' while account.remove runs) and whether
// the account is held for a reset or a relaunch.
export interface EngineAccountRoute {
  state: 'adding' | 'added' | 'removing' | null;
  held: boolean;
}

export interface EngineStatus {
  // Changes with every engine the supervisor spawns; null while none is running.
  generation: number | null;
  // Set once the engine has bound its socket and answered on stdin.
  socketPath: string | null;
  accounts: { [accountId: string]: EngineAccountRoute };
}

export type AccountEvent =
  // account.add was sent; `syncState` is the account's state in config.json then.
  | { type: 'launched'; accountId: string; syncState: string }
  | { type: 'exited'; accountId: string }
  | {
      type: 'too-many-failures';
      accountId: string;
      authFailure: boolean;
      exit: { code: number | null; signal: string | null; error: SerializedError | null };
    };

export interface SerializedError {
  message: string;
  syncError?: unknown;
}
