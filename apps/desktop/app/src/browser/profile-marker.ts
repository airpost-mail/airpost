// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import fs from 'fs';
import path from 'path';

/*
Marks a profile directory as Tern's, so "Reset Accounts and Settings" (profile-reset.ts) only
ever empties a directory Tern owns. main.js marks the default profile, and a --config-dir-path
that Tern created or found empty; a --config-dir-path that already held other files is never
marked. Kept separate from profile-reset.ts, which loads intl and so can't run before `ready`.
*/
export const TERN_PROFILE_MARKER = '.tern-profile';

export function hasTernProfileMarker(configDirPath: string) {
  return fs.existsSync(path.join(configDirPath, TERN_PROFILE_MARKER));
}

export function markTernProfile(configDirPath: string) {
  if (hasTernProfileMarker(configDirPath)) return;
  try {
    fs.writeFileSync(
      path.join(configDirPath, TERN_PROFILE_MARKER),
      'Marks this directory as a Tern profile. Reset Accounts and Settings only empties a marked directory.\n'
    );
  } catch (err) {
    console.error(`Could not mark ${configDirPath} as a Tern profile: ${err}`);
  }
}
