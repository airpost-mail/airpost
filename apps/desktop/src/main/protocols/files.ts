import { net, protocol } from 'electron';
import path from 'path';
import { pathToFileURL } from 'url';
import { FilesScheme, resolveFilesSchemeURL } from '../../shared/files-scheme';

// Serves tern-file: URLs from <configDir>/files and nowhere else.
export function registerFilesProtocol(configDirPath: string) {
  const filesDir = path.join(configDirPath, 'files');
  protocol.handle(FilesScheme, (request) => {
    if (request.method !== 'GET') {
      return new Response('Method Not Allowed', { status: 405 });
    }
    const filePath = resolveFilesSchemeURL(filesDir, request.url);
    if (!filePath) {
      return new Response('Not Found', { status: 404 });
    }
    // net.fetch on file: sets Content-Type from the extension, which SVG images need.
    return net.fetch(pathToFileURL(filePath).toString());
  });
}
