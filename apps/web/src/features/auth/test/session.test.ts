import { isRedirect } from '@tanstack/react-router';
import { describe, expect, it, vi } from 'vitest';
import { createSessionStore, DEMO_SESSION, displayName } from '../session';
import { AuthError, createMockAuthTransport } from '../transport';
import { fakeTransport, ME, MemoryStorage } from './helpers';

const URL_ = 'https://tern.example.com';

function setup(opts: { demo?: boolean; transport?: ReturnType<typeof fakeTransport> } = {}) {
  const storage = new MemoryStorage();
  const transport = opts.transport ?? fakeTransport();
  const store = createSessionStore({
    storage,
    transport,
    demo: opts.demo ? DEMO_SESSION : null,
  });
  return { storage, transport, store };
}

describe('session store', () => {
  it('starts signed out when a login is required', () => {
    expect(setup().store.get()).toBeNull();
  });

  it('starts with the demo session for the mock engine', () => {
    expect(setup({ demo: true }).store.get()?.method).toBe('demo');
  });

  it('treats corrupt storage as nothing stored', () => {
    const storage = new MemoryStorage();
    storage.setItem('tern.session', '{not json');
    const store = createSessionStore({ storage, transport: fakeTransport(), demo: DEMO_SESSION });
    expect(store.get()).toEqual(DEMO_SESSION);
  });

  it('signs in with a password, stores the token and never the password', async () => {
    const { store, storage, transport } = setup();
    const seen = vi.fn();
    store.subscribe(seen);

    const s = await store.signIn({
      method: 'password',
      serverUrl: 'tern.example.com/',
      email: ' ada@example.com ',
      password: 'correct horse battery staple',
    });

    expect(transport.login).toHaveBeenCalledWith(
      URL_,
      'ada@example.com',
      'correct horse battery staple',
      'Tern web',
    );
    expect(transport.me).toHaveBeenCalledWith(URL_, 'tern_abc');
    expect(s).toMatchObject({
      server: { url: URL_, serverId: 'srv1' },
      user: { id: 'u1', name: 'Ada Lovelace', email: 'ada@example.com' },
      token: 'tern_abc',
      method: 'password',
    });
    expect(store.get()).toEqual(s);
    expect(seen).toHaveBeenCalled();
    expect(storage.dump()).toContain('tern_abc');
    expect(storage.dump()).not.toContain('correct horse');
  });

  it('persists across a reload', async () => {
    const { store, storage } = setup();
    await store.signIn({ method: 'device-token', serverUrl: URL_, token: 'tern_xyz' });
    const reloaded = createSessionStore({ storage, transport: fakeTransport(), demo: null });
    expect(reloaded.get()?.token).toBe('tern_xyz');
  });

  it('checks a device token against /tern/v1/me', async () => {
    const { store, transport } = setup();
    await store.signIn({ method: 'device-token', serverUrl: URL_, token: ' tern_xyz ' });
    expect(transport.login).not.toHaveBeenCalled();
    expect(transport.me).toHaveBeenCalledWith(URL_, 'tern_xyz');
    expect(store.get()?.method).toBe('device-token');
  });

  it('rejects tokens without the tern_ prefix and ones the server refuses', async () => {
    const transport = fakeTransport();
    transport.me.mockRejectedValue(new AuthError('unauthorized', 'nope'));
    const { store } = setup({ transport });
    await expect(
      store.signIn({ method: 'device-token', serverUrl: URL_, token: 'abc' }),
    ).rejects.toMatchObject({ code: 'invalid-credentials' });
    await expect(
      store.signIn({ method: 'device-token', serverUrl: URL_, token: 'tern_gone' }),
    ).rejects.toMatchObject({ code: 'invalid-credentials' });
    expect(store.get()).toBeNull();
  });

  it('refuses plain http to a remote host', async () => {
    const { store, transport } = setup();
    await expect(
      store.signIn({
        method: 'password',
        serverUrl: 'http://tern.example.com',
        email: 'a@b',
        password: 'x',
      }),
    ).rejects.toBeInstanceOf(AuthError);
    expect(transport.login).not.toHaveBeenCalled();
  });

  it('accepts any credentials with the mock transport', async () => {
    const store = createSessionStore({
      storage: new MemoryStorage(),
      transport: createMockAuthTransport(0),
      demo: DEMO_SESSION,
    });
    const s = await store.signIn({
      method: 'password',
      serverUrl: 'http://localhost:5173',
      email: 'sam.rivers@example.org',
      password: '',
    });
    expect(s.user).toMatchObject({ email: 'sam.rivers@example.org', name: 'Sam Rivers' });
    expect(s.token).toMatch(/^tern_mock_/);
  });

  it('signs out, revokes the token and keeps the demo session away', async () => {
    const { store, storage, transport } = setup({ demo: true });
    await store.signIn({ method: 'device-token', serverUrl: URL_, token: 'tern_xyz' });
    await store.signOut();
    expect(transport.revoke).toHaveBeenCalledWith(URL_, 'tern_xyz');
    expect(store.get()).toBeNull();
    expect(store.notice()).toBe('user');
    expect(storage.dump()).not.toContain('tern_xyz');
    const reloaded = createSessionStore({ storage, transport, demo: DEMO_SESSION });
    expect(reloaded.get()).toBeNull();
  });

  /** A stored session read back as on the next page load, with a fresh transport. */
  async function reloadSignedIn() {
    const { store, storage } = setup();
    await store.signIn({ method: 'device-token', serverUrl: URL_, token: 'tern_xyz' });
    const transport = fakeTransport();
    return { transport, store: createSessionStore({ storage, transport, demo: null }) };
  }

  it('signs out locally when the server revokes the token', async () => {
    const { store, transport } = await reloadSignedIn();
    transport.me.mockRejectedValueOnce(new AuthError('unauthorized', 'gone'));

    await store.revalidate();

    expect(store.get()).toBeNull();
    expect(store.notice()).toBe('revoked');
    expect(transport.revoke).not.toHaveBeenCalled();
  });

  it('keeps the session on network errors', async () => {
    const { store, transport } = await reloadSignedIn();
    transport.me.mockRejectedValueOnce(new AuthError('unreachable', 'down'));
    await store.revalidate();
    expect(store.get()?.token).toBe('tern_xyz');
  });

  it('refreshes the profile once per load', async () => {
    const { store, transport } = await reloadSignedIn();
    transport.me.mockResolvedValueOnce({ ...ME, firstName: 'Augusta' });
    await store.revalidate();
    await store.revalidate();
    expect(store.get()?.user.name).toBe('Augusta Lovelace');
    expect(transport.me).toHaveBeenCalledTimes(1);
  });

  it('picks up a sign-out from another tab', async () => {
    const { store, storage } = setup();
    await store.signIn({ method: 'device-token', serverUrl: URL_, token: 'tern_xyz' });
    storage.setItem('tern.session', JSON.stringify({ v: 1, session: null }));
    const seen = vi.fn();
    store.subscribe(seen);
    store.syncFromStorage({ key: 'tern.session' });
    expect(store.get()).toBeNull();
    expect(seen).toHaveBeenCalled();
  });
});

describe('requireSession', () => {
  it('redirects to /login with the current location', () => {
    const { store } = setup();
    let thrown: unknown;
    try {
      store.requireSession({ location: { href: '/acc1/inbox/t1?x=1' } });
    } catch (err) {
      thrown = err;
    }
    expect(isRedirect(thrown)).toBe(true);
    expect((thrown as { options: unknown }).options).toMatchObject({
      to: '/login',
      search: { redirect: '/acc1/inbox/t1?x=1' },
    });
  });

  it('lets a signed-in user through and revalidates once per load', async () => {
    const { store, storage } = setup();
    await store.signIn({ method: 'device-token', serverUrl: URL_, token: 'tern_xyz' });
    const transport = fakeTransport();
    const reloaded = createSessionStore({ storage, transport, demo: null });
    expect(() => reloaded.requireSession({ location: { href: '/' } })).not.toThrow();
    expect(() => reloaded.requireSession({ location: { href: '/x' } })).not.toThrow();
    expect(transport.me).toHaveBeenCalledTimes(1);
  });
});

describe('displayName', () => {
  it('falls back to the email local part', () => {
    expect(displayName('', '', 'maya.okafor@gmail.com')).toBe('Maya Okafor');
    expect(displayName('Ada', '', 'x@y')).toBe('Ada');
  });
});
