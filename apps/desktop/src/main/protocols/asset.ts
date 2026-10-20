import { protocol } from 'electron';
import fs from 'fs';
import path from 'path';
import { bundledAssetRoots } from '../../packages/bundled-packages';

// Handles requests with 'mailspring' protocol.
//
// It's created by {Application} upon instantiation and is used to create a
// custom resource loader for 'mailspring://' URLs.
//
// The following directories are searched in order:
//   * <config-dir>/assets
//   * <config-dir>/dev/packages (unless in safe mode)
//   * <config-dir>/packages
//   * RESOURCE_PATH/src/features, then RESOURCE_PATH/{plugins,themes}
//
const FontExtensions = ['.woff2', '.woff', '.ttf', '.otf'];

export default class AssetProtocolHandler {
  loadPaths: string[] = [];

  constructor({ configDirPath, resourcePath, safeMode }) {
    if (!safeMode) {
      this.loadPaths.push(path.resolve(path.join(configDirPath, 'dev', 'packages')));
    }
    this.loadPaths.push(path.resolve(path.join(configDirPath, 'packages')));
    for (const root of bundledAssetRoots(resourcePath)) {
      this.loadPaths.push(path.resolve(root));
    }

    this.registerProtocol();
  }

  // Creates the `mailspring:` asset protocol handler.
  registerProtocol() {
    const scheme = 'mailspring';

    protocol.handle(scheme, (request) => {
      const relativePath = path.normalize(request.url.substr(scheme.length + 1));

      let filePath = null;
      for (const loadPath of this.loadPaths) {
        // Use path.join (not path.resolve) so absolute-looking inputs like
        // "/foo" stay anchored to the load path instead of replacing it.
        const candidate = path.resolve(path.join(loadPath, relativePath));
        // Ensure the resolved path is contained within the load path.
        // Append path.sep to prevent prefix-matching attacks (e.g. /packages-evil/).
        if (candidate !== loadPath && !candidate.startsWith(loadPath + path.sep)) {
          continue;
        }
        let fileStats: fs.Stats | false = false;
        try {
          fileStats = fs.statSync(candidate);
        } catch (e) {
          // path doesn't exist
        }
        if (fileStats && fileStats.isFile && fileStats.isFile()) {
          filePath = candidate;
          break;
        }
      }

      if (filePath) {
        // Fonts load in CORS mode, and the message frame (src/mail/frame) is cross-origin.
        const headers = FontExtensions.includes(path.extname(filePath).toLowerCase())
          ? { 'Access-Control-Allow-Origin': '*' }
          : undefined;
        return new Response(fs.readFileSync(filePath), { status: 200, headers });
      } else {
        return new Response('Not Found', { status: 404 });
      }
    });
  }
}
