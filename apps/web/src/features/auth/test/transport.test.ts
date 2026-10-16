import { afterEach, describe, expect, it, vi } from 'vitest';
import { createHttpGatewayTransport, createMockGatewayTransport } from '../transport';
import { SESSION } from './helpers';
import { MemoryStorage } from './memory-storage';

afterEach(() => vi.unstubAllGlobals());

function stubFetch(res: () => Response) {
  const fetchMock = vi.fn(async (..._args: [string, RequestInit?]) => res());
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

const json = (body: unknown, status = 200, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', ...headers },
  });

describe('http gateway transport', () => {
  it('reads the session on the same origin with the cookie and no token', async () => {
    const fetchMock = stubFetch(() => json(SESSION));
    expect(await createHttpGatewayTransport().session()).toEqual(SESSION);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('/gateway/v1/session');
    expect(init).toMatchObject({ credentials: 'same-origin', cache: 'no-store' });
    expect(JSON.stringify(init)).not.toMatch(/authorization/i);
  });

  it('maps 401 on the session to unpaired', async () => {
    stubFetch(() => new Response(null, { status: 401 }));
    await expect(createHttpGatewayTransport().session()).rejects.toMatchObject({
      code: 'unpaired',
    });
  });

  it('maps a network failure to unreachable', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => {
        throw new TypeError('Failed to fetch');
      }),
    );
    await expect(createHttpGatewayTransport().session()).rejects.toMatchObject({
      code: 'unreachable',
    });
  });

  it('posts the code and device name as JSON', async () => {
    const fetchMock = stubFetch(() => json({ device: SESSION.device }));
    const device = await createHttpGatewayTransport().pair('ABCDEFGHJKLM', 'Laptop');
    expect(device).toEqual(SESSION.device);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('/gateway/v1/pair');
    expect(init?.method).toBe('POST');
    expect(JSON.parse(init?.body as string)).toEqual({
      code: 'ABCDEFGHJKLM',
      deviceName: 'Laptop',
    });
  });

  it('explains refused codes and rate limits', async () => {
    const t = createHttpGatewayTransport();
    stubFetch(() => new Response(null, { status: 401 }));
    await expect(t.pair('ABCDEFGHJKLM', 'x')).rejects.toMatchObject({ code: 'invalid-code' });
    stubFetch(() => new Response(null, { status: 400 }));
    await expect(t.pair('ABCDEFGHJKLM', 'x')).rejects.toMatchObject({ code: 'invalid-code' });
    stubFetch(() => new Response(null, { status: 429, headers: { 'Retry-After': '30' } }));
    await expect(t.pair('ABCDEFGHJKLM', 'x')).rejects.toMatchObject({
      code: 'rate-limited',
      retryAfterSecs: 30,
      message: 'Too many attempts. Try again in 30 s.',
    });
  });

  it('unpairs with a POST and swallows failures', async () => {
    const fetchMock = stubFetch(() => new Response(null, { status: 204 }));
    await createHttpGatewayTransport().unpair();
    expect(fetchMock.mock.calls[0][0]).toBe('/gateway/v1/unpair');
    expect(fetchMock.mock.calls[0][1]?.method).toBe('POST');
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => {
        throw new TypeError('offline');
      }),
    );
    await expect(createHttpGatewayTransport().unpair()).resolves.toBeUndefined();
  });
});

describe('mock gateway transport', () => {
  it('starts paired, unpairs and pairs again, storing nothing token-like', async () => {
    const storage = new MemoryStorage();
    const t = createMockGatewayTransport(storage, 0);
    expect((await t.session()).device.name).toBe('Demo browser');
    await t.unpair();
    await expect(t.session()).rejects.toMatchObject({ code: 'unpaired' });
    await t.pair('ABCDEFGHJKLM', 'Laptop');
    expect((await t.session()).device.name).toBe('Laptop');
    expect(storage.dump()).not.toMatch(/token|ABCDEFGHJKLM/i);
  });
});
