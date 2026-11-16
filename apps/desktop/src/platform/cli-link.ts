// SPDX-License-Identifier: GPL-3.0-only

// The `airpost` command on the PATH (Preferences > General): a symlink to the bundled binary that
// main makes (main/cli-link.ts), as VS Code and Zed install `code` and `zed`, or on a profile
// other than the default one a wrapper that sets AIRPOST_CONFIG_DIR.
export const CliLinkChannel = {
  // invoke -> CliLinkStatus
  Status: 'cli-link:status',
  // invoke(replace: boolean) -> CliLinkResult. Asks for an administrator password when the
  // folder needs one.
  Install: 'cli-link:install',
  // invoke -> CliLinkResult
  Uninstall: 'cli-link:uninstall',
} as const;

export type CliLinkState =
  // `path` is a link to this copy's airpost, or its wrapper for this profile.
  | 'installed'
  // Nothing at `path`; installing links it there.
  | 'missing'
  // Something else is at `path`: a link or Airpost wrapper running `otherTarget` (replaceable),
  // or another program.
  | 'other'
  // Windows, or this copy has no airpost.
  | 'unsupported';

export interface CliLinkStatus {
  state: CliLinkState;
  binary: string | null;
  path: string | null;
  /** The profile the command pins, when it isn't the one `airpost` finds on its own. */
  profile?: string;
  otherTarget?: string;
  /** The profile another Airpost wrapper pins; null for a link. */
  otherProfile?: string | null;
  /** The thing at `path` runs this copy's airpost, with another profile. */
  sameBinary?: boolean;
  replaceable?: boolean;
  /** A packaged macOS copy outside /Applications, whose link would break once it's moved. */
  needsMove?: boolean;
}

export interface CliLinkResult {
  ok: boolean;
  cancelled?: boolean;
  error?: string;
  status: CliLinkStatus;
}
