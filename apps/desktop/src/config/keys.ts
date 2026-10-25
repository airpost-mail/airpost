// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

// config.json: the ids of accounts deleted in the app that the engine may still have
// registered, until its account.remove answers. The main process's EngineSupervisor
// and the windows' AccountStore both read it.
export const CONFIG_ACCOUNTS_TO_REMOVE_KEY = 'accountsToRemoveFromEngine';
