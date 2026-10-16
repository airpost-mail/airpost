import { afterEach, describe, expect, it, vi } from 'vitest';
import { createHttpAuthTransport, parseLoginResultPage } from '../transport';

const RESULT = {
  type: 'tern-login',
  version: 1,
  token: 'tern_abc',
  userId: 'u1',
  serverId: 'srv1',
  email: 'ada@example.com',
};

function resultPage(json: object) {
  const b64 = btoa(JSON.stringify(json)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  return `<!doctype html><p>Signed in.</p><div id="tern-login-result" hidden>${b64}</div>`;
}

afterEach(() => vi.unstubAllGlobals());

describe('parseLoginResultPage', () => {
  it('decodes the base64url result', () => {
    expect(parseLoginResultPage(resultPage(RESULT))).toEqual(RESULT);
  });

  it('ignores pages without a valid result', () => {
    expect(parseLoginResultPage('<form>Wrong password</form>')).toBeNull();
    expect(parseLoginResultPage(resultPage({ ...RESULT, type: 'other' }))).toBeNull();
  });
});

describe('http auth transport', () => {
  it('posts the login form and reads the token from the page', async () => {
    const fetchMock = vi.fn(async () => new Response(resultPage(RESULT), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);

    const r = await createHttpAuthTransport().login(
      'https://t.example',
      'ada@example.com',
      'pw',
      'Tern web',
    );

    expect(r.token).toBe('tern_abc');
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('https://t.example/tern/v1/login');
    expect(init.method).toBe('POST');
    expect(String(init.body)).toBe(
      'email=ada%40example.com&password=pw&action=login&device=Tern+web',
    );
  });

  it('maps a form page without a result to invalid credentials, and 429 to rate-limited', async () => {
    const t = createHttpAuthTransport();
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('<form></form>', { status: 200 })),
    );
    await expect(t.login('https://t.example', 'a@b', 'x', 'd')).rejects.toMatchObject({
      code: 'invalid-credentials',
    });
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('', { status: 429 })),
    );
    await expect(t.login('https://t.example', 'a@b', 'x', 'd')).rejects.toMatchObject({
      code: 'rate-limited',
    });
  });

  it('sends the token as a bearer header and maps 401', async () => {
    const fetchMock = vi.fn(async () => new Response('', { status: 401 }));
    vi.stubGlobal('fetch', fetchMock);
    await expect(
      createHttpAuthTransport().me('https://t.example', 'tern_abc'),
    ).rejects.toMatchObject({
      code: 'unauthorized',
    });
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('https://t.example/tern/v1/me');
    expect(init.headers).toEqual({ Authorization: 'Bearer tern_abc' });
  });

  it('reports an unreachable server', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => Promise.reject(new TypeError('Failed to fetch'))),
    );
    await expect(createHttpAuthTransport().server('https://t.example')).rejects.toMatchObject({
      code: 'unreachable',
    });
  });
});
