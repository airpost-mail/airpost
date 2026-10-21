// SPDX-License-Identifier: GPL-3.0-only

// The `tern` command on the PATH (Preferences > Agents): a symlink to the bundled binary that
// main makes (main/cli-link.ts), as VS Code and Zed install `code` and `zed`.
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
  // `path` is a link to this copy's tern.
  | 'installed'
  // Nothing at `path`; installing links it there.
  | 'missing'
  // Something else is at `path`: a link to `otherTarget` (replaceable), or a real file.
  | 'other'
  // Windows, or this copy has no tern.
  | 'unsupported';

export interface CliLinkStatus {
  state: CliLinkState;
  binary: string | null;
  path: string | null;
  otherTarget?: string;
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
