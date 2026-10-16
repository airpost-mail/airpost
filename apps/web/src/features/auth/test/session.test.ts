import { isRedirect } from '@tanstack/react-router';
import { describe, expect, it, vi } from 'vitest';
import { createSessionStore } from '../session';
import { AuthError } from '../transport';
import { DEVICE, fakeTransport, SESSION, unpaired, unreachable } from './helpers';

function setup() {
  const transport = fakeTransport();
  return { transport, store: createSessionStore({ transport }) };
}

describe('session store', () => {
  it('starts unknown, then loads the session once for every caller', async () => {
    const { store, transport } = setup();
    expect(store.snapshot()).toEqual({ status: 'unknown', session: null });
    await Promise.all([store.load(), store.load()]);
    expect(transport.session).toHaveBeenCalledTimes(1);
    expect(store.snapshot().status).toBe('paired');
    expect(store.get()).toMatchObject({ device: DEVICE, demo: false });
  });

  it('reports engine health from the session route', async () => {
    const { store, transport } = setup();
    expect(await store.refresh()).toEqual({ ok: true, engineUp: true });
    transport.session.mockResolvedValueOnce({ ...SESSION, engine: { state: 'down' } });
    expect(await store.refresh()).toEqual({ ok: true, engineUp: false });
    expect(store.get()?.engine.state).toBe('down');
  });

  it('keeps the last session while the gateway is unreachable', async () => {
    const { store, transport } = setup();
    await store.load();
    transport.session.mockRejectedValueOnce(unreachable());
    expect(await store.refresh()).toEqual({ ok: false, reason: 'unreachable' });
    expect(store.snapshot().status).toBe('unreachable');
    expect(store.get()?.device.id).toBe('dev1');
  });

  it('drops the session on 401 and says the gateway did it', async () => {
    const { store, transport } = setup();
    await store.load();
    transport.session.mockRejectedValueOnce(unpaired());
    expect(await store.refresh()).toEqual({ ok: false, reason: 'unpaired' });
    expect(store.snapshot()).toEqual({ status: 'unpaired', session: null });
    expect(store.notice()).toBe('revoked');
  });

  it('shows no notice for a browser that was never paired', async () => {
    const { store, transport } = setup();
    transport.session.mockRejectedValueOnce(unpaired());
    await store.load();
    expect(store.snapshot().status).toBe('unpaired');
    expect(store.notice()).toBeNull();
  });

  it('pairs with a normalized code and a trimmed name', async () => {
    const { store, transport } = setup();
    transport.session.mockRejectedValueOnce(unpaired());
    await store.load();
    const seen = vi.fn();
    store.subscribe(seen);

    const s = await store.pair(' abcd-efgh jklm ', '  Laptop  ');

    expect(transport.pair).toHaveBeenCalledWith('ABCDEFGHJKLM', 'Laptop');
    expect(s.device.id).toBe('dev1');
    expect(store.snapshot().status).toBe('paired');
    expect(seen).toHaveBeenCalled();
  });

  it('refuses a malformed code without calling the gateway', async () => {
    const { store, transport } = setup();
    await expect(store.pair('ABC', 'x')).rejects.toMatchObject({ code: 'invalid-code' });
    expect(transport.pair).not.toHaveBeenCalled();
  });

  it('passes the gateway refusal through', async () => {
    const { store, transport } = setup();
    transport.pair.mockRejectedValueOnce(new AuthError('invalid-code', 'used'));
    await expect(store.pair('ABCDEFGHJKLM', 'x')).rejects.toThrow('used');
  });

  it('signs out locally even when the gateway call fails', async () => {
    const { store, transport } = setup();
    await store.load();
    transport.unpair.mockRejectedValueOnce(new Error('network'));
    await store.signOut().catch(() => undefined);
    expect(store.snapshot()).toEqual({ status: 'unpaired', session: null });
    expect(store.notice()).toBe('user');
  });

  it('redirects an unpaired browser to /pair, and a paired one away from it', async () => {
    const { store, transport } = setup();
    transport.session.mockRejectedValueOnce(unpaired());
    const away = await store.requireSession({ location: { href: '/inbox?x=1' } }).catch((e) => e);
    expect(isRedirect(away)).toBe(true);
    expect(away.options).toMatchObject({ to: '/pair', search: { redirect: '/inbox?x=1' } });

    await store.pair('ABCDEFGHJKLM', 'x');
    const back = await store.redirectIfPaired({ search: { redirect: '/inbox' } }).catch((e) => e);
    expect(isRedirect(back)).toBe(true);
    expect(back.options).toMatchObject({ href: '/inbox' });
  });

  it('lets the app load while the gateway is unreachable', async () => {
    const { store, transport } = setup();
    transport.session.mockRejectedValueOnce(unreachable());
    await expect(store.requireSession({ location: { href: '/' } })).resolves.toBeUndefined();
  });
});
