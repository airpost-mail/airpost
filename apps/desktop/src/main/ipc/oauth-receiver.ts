// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The OAuth redirect receiver. The engine starts a
sign-in (`account.oauth.begin`) and keeps its PKCE verifier and client secret; the browser
comes back to http://127.0.0.1:<port> (Google) or http://localhost:<port>/desktop
(Microsoft), and this server, in the main process and bound to 127.0.0.1 only, hands the
`code` and `state` to the onboarding window, which passes them to `account.oauth.complete`.
The window checks nothing itself: the receiver drops a redirect whose `state` isn't the one
`begin` returned, and the engine checks it again.

One sign-in at a time: a new one (Try Again, another window) ends the one before it.
*/
import http from 'http';
import crypto from 'crypto';
import url from 'url';
import { localized } from '../../intl/localize';
import { OAuthReceiverChannel, OAuthRedirect } from '../../engine/channels';

export const OAUTH_RECEIVER_HOST = '127.0.0.1';
// How long a sign-in may wait for the browser.
const RECEIVE_TIMEOUT_MS = 15 * 60 * 1000;

// Reads a query parameter, ignoring the path so one listener serves Google (root) and
// Microsoft (`/desktop`). decodeURIComponent keeps `+` literal (RFC 3986), as providers send
// it in codes.
export function queryParam(requestUrl: string, param: string): string | null {
  const rawQuery = url.parse(requestUrl || '').query || '';
  const match = rawQuery.match(new RegExp(`(?:^|&)${param}=([^&]*)`));
  if (!match) return null;
  try {
    return decodeURIComponent(match[1]);
  } catch {
    return null;
  }
}

export function stateMatches(received: string | null, expected: string | null): boolean {
  if (!received || !expected) return false;
  const a = Buffer.from(received, 'utf8');
  const b = Buffer.from(expected, 'utf8');
  return a.length === b.length && crypto.timingSafeEqual(a, b);
}

function page(message: string) {
  const title = localized('Tern');
  return (
    `<!DOCTYPE html><html><head><meta charset="utf-8"><title>${title}</title></head>` +
    `<body style="font-family: sans-serif; text-align: center; margin-top: 80px;">` +
    `${message}</body></html>`
  );
}

interface Pending {
  server: http.Server;
  state: string;
  finish: (r: OAuthRedirect) => void;
}

let current: Pending | null = null;

// Ends the sign-in in progress (the one waiting for `state`, when given), answering its
// window with `error`.
export function cancelReceive(error = 'cancelled', state?: string) {
  const pending = current;
  if (!pending || (state !== undefined && pending.state !== state)) return;
  current = null;
  if (pending) {
    pending.server.close();
    pending.finish({ error });
  }
}

// Listens on 127.0.0.1:`port` until the redirect carrying `state` comes in, or the provider
// reports an error, or `timeoutMs` passes. Resolves; never rejects.
export function receive(
  { port, state }: { port: number; state: string },
  timeoutMs = RECEIVE_TIMEOUT_MS
): Promise<OAuthRedirect> {
  cancelReceive('superseded');
  return new Promise<OAuthRedirect>((resolve) => {
    let done = false;
    let timer: NodeJS.Timeout = null;
    const server = http.createServer((request, response) => {
      const code = queryParam(request.url, 'code');
      const error = queryParam(request.url, 'error');
      const received = queryParam(request.url, 'state');
      if (!code && !error) {
        response.writeHead(404);
        response.end('Unknown Request');
        return;
      }
      if (!stateMatches(received, state)) {
        // A tab left open from an earlier sign-in: this one keeps waiting.
        response.writeHead(400, { 'Content-Type': 'text/html; charset=utf-8' });
        response.end(
          page(
            localized('This sign-in response is from an earlier attempt. Go back and try again.')
          )
        );
        return;
      }
      response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
      if (error) {
        response.end(page(localized('The sign-in was cancelled. You can close this window.')));
        finish({ error: `denied: ${error}` });
      } else {
        response.end(page(localized('You can close this window and return to the app.')));
        finish({ code, state: received });
      }
    });
    const finish = (r: OAuthRedirect) => {
      if (done) return;
      done = true;
      if (timer) clearTimeout(timer);
      if (current && current.server === server) current = null;
      server.close();
      resolve(r);
    };
    current = { server, state, finish };
    server.once('error', (err: NodeJS.ErrnoException) =>
      finish({ error: `listen: ${err.code || err.message}` })
    );
    timer = setTimeout(() => finish({ error: 'timeout' }), timeoutMs);
    server.listen(port, OAUTH_RECEIVER_HOST);
  });
}

export function registerOAuthReceiverIPC(ipcMain: Electron.IpcMain) {
  ipcMain.handle(
    OAuthReceiverChannel.Receive,
    (_event, params: { port: number; state: string }) => {
      if (!params || !Number.isInteger(params.port) || typeof params.state !== 'string') {
        return { error: 'invalid' } as OAuthRedirect;
      }
      return receive(params);
    }
  );
  ipcMain.handle(OAuthReceiverChannel.Cancel, (_event, state?: string) =>
    cancelReceive('cancelled', typeof state === 'string' ? state : undefined)
  );
}
