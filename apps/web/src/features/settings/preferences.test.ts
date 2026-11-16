// Tests the shared preferences store in src/lib, which the General page writes.
import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

type PrefsModule = typeof import('@/lib/preferences');

async function freshModule(): Promise<PrefsModule> {
  vi.resetModules();
  return import('@/lib/preferences');
}

beforeEach(() => {
  localStorage.clear();
  delete document.documentElement.dataset.density;
});

describe('preferences store', () => {
  it('starts from the defaults', async () => {
    const { getPreferences } = await freshModule();
    expect(getPreferences()).toEqual({
      density: 'compact',
      loadRemoteImages: false,
      markReadOnOpen: true,
      readingPane: 'right',
      markReadDelayMs: 500,
      undoSendMs: 5000,
    });
    expect(document.documentElement.dataset.density).toBe('compact');
  });

  it('persists changes and applies density to <html>', async () => {
    const { setPreferences, getPreferences } = await freshModule();
    setPreferences({ density: 'comfortable', undoSendMs: 30000, readingPane: 'bottom' });
    expect(getPreferences()).toMatchObject({ density: 'comfortable', undoSendMs: 30000 });
    expect(JSON.parse(localStorage.getItem('airpost.preferences')!)).toMatchObject({
      density: 'comfortable',
      undoSendMs: 30000,
      readingPane: 'bottom',
    });
    expect(document.documentElement.dataset.density).toBe('comfortable');
  });

  it('merges saved values over defaults, so new fields get their default', async () => {
    localStorage.setItem('airpost.preferences', JSON.stringify({ density: 'comfortable' }));
    const { getPreferences } = await freshModule();
    expect(getPreferences()).toMatchObject({
      density: 'comfortable',
      markReadDelayMs: 500,
      readingPane: 'right',
    });
  });

  it('falls back to defaults on corrupt storage', async () => {
    localStorage.setItem('airpost.preferences', '{not json');
    const { getPreferences } = await freshModule();
    expect(getPreferences().density).toBe('compact');
  });

  it('notifies subscribers', async () => {
    const { usePreferences, setPreferences } = await freshModule();
    const { result } = renderHook(() => usePreferences());
    expect(result.current.loadRemoteImages).toBe(false);
    act(() => setPreferences({ loadRemoteImages: true }));
    expect(result.current.loadRemoteImages).toBe(true);
  });
});
