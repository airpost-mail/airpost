import http from 'http';
import {
  cancelReceive,
  OAUTH_RECEIVER_HOST,
  queryParam,
  receive,
  stateMatches,
} from '../../src/main/ipc/oauth-receiver';

// A fake http.Server: records listen() and lets the spec send requests to the handler.
function fakeServer() {
  let handler: (req: any, res: any) => void = null;
  const server: any = {
    listen: jasmine.createSpy('listen'),
    once: () => server,
    close: jasmine.createSpy('close'),
  };
  spyOn(http, 'createServer').andCallFake((h) => {
    handler = h;
    return server;
  });
  const request = (url: string) => {
    const res: any = { status: 200, body: '' };
    res.writeHead = (status: number) => (res.status = status);
    res.end = (body: string) => (res.body = body);
    handler({ url }, res);
    return res;
  };
  return { server, request };
}

describe('the OAuth redirect receiver', function oauthReceiverSpecs() {
  it('reads codes as providers send them', () => {
    expect(queryParam('/?code=4%2F0AX4XfWiTest', 'code')).toEqual('4/0AX4XfWiTest');
    // A literal + stays one (RFC 3986), %2B too.
    expect(queryParam('/?code=4%2F0AeanS0m+test', 'code')).toEqual('4/0AeanS0m+test');
    expect(queryParam('/?code=a%2Bb', 'code')).toEqual('a+b');
    expect(queryParam('/desktop?code=M.R3&state=abc', 'state')).toEqual('abc');
    expect(queryParam('/?code=4%2F0AeanS0m%ZZbad', 'code')).toEqual(null);
    expect(queryParam('/', 'code')).toEqual(null);
  });

  it('compares states in full', () => {
    expect(stateMatches('abc', 'abc')).toBe(true);
    expect(stateMatches('abd', 'abc')).toBe(false);
    expect(stateMatches('abcx', 'abc')).toBe(false);
    expect(stateMatches(null, 'abc')).toBe(false);
  });

  it('listens on 127.0.0.1 and hands over the redirect with its state only', async () => {
    const { server, request } = fakeServer();
    const done = receive({ port: 12141, state: 'st8' });
    expect(server.listen).toHaveBeenCalledWith(12141, '127.0.0.1');
    expect(OAUTH_RECEIVER_HOST).toEqual('127.0.0.1');

    expect(request('/favicon.ico').status).toEqual(404);
    // A tab from an earlier sign-in: refused, and this one keeps waiting.
    expect(request('/?code=old&state=earlier').status).toEqual(400);
    expect(request('/desktop?code=M.R3%2Bx&state=st8').status).toEqual(200);
    expect(await done).toEqual({ code: 'M.R3+x', state: 'st8' });
    expect(server.close).toHaveBeenCalled();
  });

  it('reports a refused sign-in, and a cancel', async () => {
    const { request } = fakeServer();
    const refused = receive({ port: 12141, state: 's1' });
    request('/?error=access_denied&state=s1');
    expect(await refused).toEqual({ error: 'denied: access_denied' });

    const cancelled = receive({ port: 12141, state: 's2' });
    cancelReceive('cancelled', 'another');
    const later = receive({ port: 12141, state: 's3' });
    expect(await cancelled).toEqual({ error: 'superseded' });
    cancelReceive('cancelled', 's3');
    expect(await later).toEqual({ error: 'cancelled' });
  });
});
