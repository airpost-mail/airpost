// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import path from 'path';
import { fileURLToPath } from 'url';

// In a file: document CSP 'self' and `file:` match every file: URL, including ones that name
// a host. Chromium on Windows opens file://host/share/... as the UNC path \\host\share\...,
// which connects to the host over SMB and offers NTLM credentials. These checks back the CSP
// from webRequest.onBeforeRequest.

/** True for a file: URL that names a host other than localhost, or a UNC path. */
export function isHostedFileURL(url: string): boolean {
  // Parsed by hand: Node's URL and Blink's disagree on file:////host/share, and this runs in
  // both. Chromium on Windows opens that form as a UNC path too.
  const match = /^file:([\\/]*)([^\\/?#]*)/i.exec(url);
  if (!match) return false;
  const [, slashes, host] = match;
  if (slashes.length >= 4) return true;
  if (slashes.length !== 2) return false;
  // file://C:/... is a drive letter, not a host.
  return host !== '' && host.toLowerCase() !== 'localhost' && !/^[a-z][:|]$/i.test(host);
}

function normalize(p: string) {
  const resolved = path.resolve(p);
  return process.platform === 'win32' ? resolved.toLowerCase() : resolved;
}

/** True for a local file: URL inside one of `roots`. */
export function isFileURLWithin(url: string, roots: string[]): boolean {
  if (!/^file:/i.test(url) || isHostedFileURL(url)) return false;
  let filePath: string;
  try {
    filePath = normalize(fileURLToPath(url));
  } catch {
    return false;
  }
  return roots.some((root) => {
    const r = normalize(root);
    return filePath === r || filePath.startsWith(r.endsWith(path.sep) ? r : r + path.sep);
  });
}
