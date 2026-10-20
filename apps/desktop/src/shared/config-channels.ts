// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
IPC between the windows' Config (src/shared/config.ts) and the main process's ConfigPersistenceManager
(src/main/config-persistence-manager.ts), which holds config.json. Reads and writes are
sendSync: AppEnv.config.get stays synchronous, and a write is read back at once.
*/
export const ConfigChannel = {
  // sendSync -> ConfigReply with `value` = the settings as JSON text.
  GetRaw: 'config:get-raw',
  // sendSync(keyPath, value) -> ConfigReply. Every other window gets Reloaded.
  SetRaw: 'config:set-raw',
  // sendSync(settings) -> ConfigReply: replaces every setting.
  SetAll: 'config:set-all',
  // main -> windows, with the settings, after another window, the main process or the engine
  // changed them.
  Reloaded: 'on-config-reloaded',
} as const;

// Errors cross IPC as text.
export interface ConfigReply {
  value?: string;
  error?: string;
}
