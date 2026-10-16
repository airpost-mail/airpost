import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { ConnectionState, EngineClient } from '@/engine';
import { EngineContext } from '@/engine/context';
import { statusFor } from '../connection';
import { ConnectionStatus } from '../ConnectionStatus';
import { OfflineBanner } from '../OfflineBanner';

function fakeEngine(initial: ConnectionState, kind: 'mock' | 'ws' = 'ws') {
  const listeners = new Set<(s: ConnectionState) => void>();
  const engine = {
    kind,
    state: initial,
    connect: vi.fn(async () => {
      engine.set('open');
      return { protocol: 1, engine: 'fake', pid: 1, accounts: [] };
    }),
    close: vi.fn(),
    call: vi.fn(),
    onNotification: () => () => {},
    onStateChange(l: (s: ConnectionState) => void) {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    fileUrl: () => null,
    set(s: ConnectionState) {
      engine.state = s;
      for (const l of listeners) l(s);
    },
  };
  return engine;
}

function renderWith(engine: ReturnType<typeof fakeEngine>, ui: React.ReactNode) {
  return render(
    <EngineContext.Provider value={engine as unknown as EngineClient}>{ui}</EngineContext.Provider>,
  );
}

describe('statusFor', () => {
  it('maps engine states to three statuses', () => {
    expect(statusFor('open', true)).toBe('connected');
    expect(statusFor('connecting', true)).toBe('connecting');
    expect(statusFor('reconnecting', true)).toBe('connecting');
    expect(statusFor('closed', true)).toBe('offline');
    expect(statusFor('idle', true)).toBe('offline');
    expect(statusFor('open', false)).toBe('offline');
  });
});

describe('ConnectionStatus', () => {
  it('follows the engine state', () => {
    const engine = fakeEngine('connecting');
    renderWith(engine, <ConnectionStatus />);
    const el = screen.getByRole('status');
    expect(el).toHaveAttribute('data-status', 'connecting');
    expect(el).toHaveAccessibleName('Engine: Connecting');

    act(() => engine.set('open'));
    expect(el).toHaveAttribute('data-status', 'connected');

    act(() => engine.set('closed'));
    expect(el).toHaveAttribute('data-status', 'offline');
    expect(el).toHaveTextContent('Disconnected');
  });

  it('reads offline when the browser has no network', () => {
    const spy = vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(false);
    renderWith(fakeEngine('open'), <ConnectionStatus />);
    expect(screen.getByRole('status')).toHaveAttribute('data-status', 'offline');
    spy.mockRestore();
  });
});

describe('OfflineBanner', () => {
  it('stays hidden while connected and during a quick connect', () => {
    vi.useFakeTimers();
    const engine = fakeEngine('connecting');
    const { container } = renderWith(engine, <OfflineBanner />);
    expect(container).toBeEmptyDOMElement();
    act(() => vi.advanceTimersByTime(500));
    act(() => engine.set('open'));
    act(() => vi.advanceTimersByTime(2000));
    expect(container).toBeEmptyDOMElement();
    vi.useRealTimers();
  });

  it('shows a slow connect after the grace period', () => {
    vi.useFakeTimers();
    renderWith(fakeEngine('connecting'), <OfflineBanner />);
    act(() => vi.advanceTimersByTime(1600));
    expect(screen.getByRole('status')).toHaveTextContent('Connecting to the engine');
    vi.useRealTimers();
  });

  it('shows reconnecting, then offline with a retry', async () => {
    const engine = fakeEngine('reconnecting');
    renderWith(engine, <OfflineBanner />);
    expect(screen.getByRole('status')).toHaveTextContent('Reconnecting');

    act(() => engine.set('closed'));
    expect(screen.getByRole('status')).toHaveTextContent("Couldn't reach the engine.");

    await userEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(engine.connect).toHaveBeenCalled();
    expect(screen.queryByRole('status')).toBeNull();
  });
});
