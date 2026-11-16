// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import fs from 'fs';
import path from 'path';

/*
Marks a profile directory as Airpost's, so "Reset Accounts and Settings" (reset.ts) only
ever empties a directory Airpost owns. main.js marks the default profile, and a --config-dir-path
that Airpost created or found empty; a --config-dir-path that already held other files is never
marked. Kept separate from reset.ts, which loads intl and so can't run before `ready`.
*/
export const AIRPOST_PROFILE_MARKER = '.airpost-profile';

export function hasAirpostProfileMarker(configDirPath: string) {
  return fs.existsSync(path.join(configDirPath, AIRPOST_PROFILE_MARKER));
}

export function markAirpostProfile(configDirPath: string) {
  if (hasAirpostProfileMarker(configDirPath)) return;
  try {
    fs.writeFileSync(
      path.join(configDirPath, AIRPOST_PROFILE_MARKER),
      'Marks this directory as an Airpost profile. Reset Accounts and Settings only empties a marked directory.\n'
    );
  } catch (err) {
    console.error(`Could not mark ${configDirPath} as an Airpost profile: ${err}`);
  }
}
