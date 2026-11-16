import fs from 'fs';
import path from 'path';

// Downloaded attachments (<configDir>/files) are shown to message documents through this
// scheme instead of file:, so the message CSP never has to allow file: (which reaches SMB
// shares on Windows). The main process serves it (main/protocols/files.ts).
export const FilesScheme = 'airpost-file';

const Prefix = `${FilesScheme}://`;

// encodeURIComponent leaves these alone, but the URL is spliced into src='…' attributes.
function encodeSegment(segment: string) {
  return encodeURIComponent(segment).replace(
    /['()*!]/g,
    (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`
  );
}

/** The airpost-file: URL for a file under `filesDir`, or null if it isn't under it. */
export function filesSchemeURL(filesDir: string, filePath: string): string | null {
  const relative = path.relative(path.resolve(filesDir), path.resolve(filePath));
  if (!relative || relative.startsWith('..') || path.isAbsolute(relative)) {
    return null;
  }
  return `${Prefix}/${relative.split(path.sep).map(encodeSegment).join('/')}`;
}

function isWithin(root: string, candidate: string) {
  return candidate.startsWith(root + path.sep);
}

/**
 * The file a airpost-file: URL names, or null unless it is a regular file inside `filesDir`
 * after resolving `..`, encoded separators and symlinks.
 */
export function resolveFilesSchemeURL(filesDir: string, url: string): string | null {
  if (typeof url !== 'string' || !url.startsWith(Prefix)) {
    return null;
  }
  const rest = url.slice(Prefix.length).split(/[?#]/)[0];
  const segments: string[] = [];
  for (const raw of rest.split('/')) {
    if (raw === '') continue;
    let segment: string;
    try {
      segment = decodeURIComponent(raw);
    } catch {
      return null;
    }
    if (
      segment === '.' ||
      segment === '..' ||
      segment.includes('/') ||
      segment.includes('\\') ||
      segment.includes('\0')
    ) {
      return null;
    }
    segments.push(segment);
  }
  if (segments.length === 0) {
    return null;
  }

  const root = path.resolve(filesDir);
  const candidate = path.resolve(root, ...segments);
  if (!isWithin(root, candidate)) {
    return null;
  }

  try {
    const realRoot = fs.realpathSync(root);
    const realCandidate = fs.realpathSync(candidate);
    if (!isWithin(realRoot, realCandidate)) {
      return null;
    }
    return fs.statSync(realCandidate).isFile() ? realCandidate : null;
  } catch {
    return null;
  }
}
