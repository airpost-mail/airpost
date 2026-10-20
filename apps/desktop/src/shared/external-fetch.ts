import { ipcRenderer } from 'electron';

export interface ExternalFetchInit {
  method?: 'GET' | 'POST';
  headers?: Record<string, string>;
  body?: string;
  redirect?: 'follow' | 'error';
}

export interface ExternalFetchResult {
  status: number;
  statusText: string;
  headers: [string, string][];
  body: Uint8Array;
}

const NullBodyStatuses = [101, 204, 205, 304];

/**
 * fetch() for http(s) URLs outside the window CSP's connect-src, run in the main process
 * (main/ipc/external-fetch.ts). Sends no cookies.
 */
export const ExternalFetch = {
  async fetch(url: string, init: ExternalFetchInit = {}): Promise<Response> {
    const result: ExternalFetchResult = await ipcRenderer.invoke('external-fetch', url, init);
    const body = NullBodyStatuses.includes(result.status) ? null : result.body;
    return new Response(body, {
      status: result.status,
      statusText: result.statusText,
      headers: result.headers,
    });
  },
};
