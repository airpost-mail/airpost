import { net } from 'electron';
import { isAppWindowContents } from '../windows/app-window';
import type { ExternalFetchInit, ExternalFetchResult } from '../../window/external-fetch';

const MaxResponseBytes = 50 * 1024 * 1024;

/**
 * Runs the renderer's requests to hosts that come from user data (autoconfig, one-click
 * unsubscribe, saving a remote image), so the window CSP's connect-src can stay pinned.
 * Only Tern windows may call it; quick preview and print windows have no access to it.
 */
export function registerExternalFetchIPC(ipcMain: Electron.IpcMain) {
  ipcMain.handle(
    'external-fetch',
    async (event, url: string, init: ExternalFetchInit = {}): Promise<ExternalFetchResult> => {
      if (!isAppWindowContents(event.sender)) {
        throw new Error('external-fetch is only available to Tern windows');
      }
      const { protocol } = new URL(url);
      if (protocol !== 'https:' && protocol !== 'http:') {
        throw new Error(`external-fetch does not fetch ${protocol} URLs`);
      }
      const method = init.method || 'GET';
      if (method !== 'GET' && method !== 'POST') {
        throw new Error(`external-fetch does not send ${method} requests`);
      }

      const response = await net.fetch(url, {
        method,
        headers: init.headers,
        body: method === 'POST' ? init.body : undefined,
        redirect: init.redirect === 'error' ? 'error' : 'follow',
        credentials: 'omit',
      });
      const body = new Uint8Array(await response.arrayBuffer());
      if (body.byteLength > MaxResponseBytes) {
        throw new Error('external-fetch response is too large');
      }
      return {
        status: response.status,
        statusText: response.statusText,
        headers: [...response.headers.entries()],
        body,
      };
    }
  );
}
