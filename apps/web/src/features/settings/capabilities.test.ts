import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import {
  CAPABILITIES,
  CAPABILITY_NAMES,
  resolveCapability,
  useCapability,
  type CapabilityContext,
} from './capabilities';
import {
  getIntegrations,
  resetIntegrations,
  saveProvider,
  setProviderEnabled,
} from './integrations/store';

const ctx = (connected = false): CapabilityContext => ({
  integrations: getIntegrations(),
  server: { connected },
});

beforeEach(() => resetIntegrations());

describe('resolveCapability', () => {
  it('has every Pro feature with the agreed source', () => {
    const bySource = (s: string) => CAPABILITY_NAMES.filter((n) => CAPABILITIES[n].source === s);
    expect(bySource('local')).toEqual([
      'snooze',
      'send-later',
      'reminders',
      'templates',
      'insights',
    ]);
    expect(bySource('integration')).toEqual(['translation', 'grammar', 'enrichment']);
    expect(bySource('server')).toEqual([
      'read-receipts',
      'link-tracking',
      'sharing',
      'metadata-sync',
    ]);
  });

  it('makes local features available with nothing configured', () => {
    for (const n of ['snooze', 'send-later', 'reminders', 'templates', 'insights'] as const) {
      expect(resolveCapability(n, ctx())).toMatchObject({ available: true, reason: 'ok' });
    }
  });

  it('needs a Tern server for server features', () => {
    expect(resolveCapability('read-receipts', ctx())).toMatchObject({
      available: false,
      reason: 'needs-server',
    });
    expect(resolveCapability('sharing', ctx(true))).toMatchObject({ available: true });
  });

  it('needs an enabled, configured provider for integration features', () => {
    expect(resolveCapability('translation', ctx()).reason).toBe('needs-integration');

    setProviderEnabled('deepl', true);
    // Enabled but no key yet.
    expect(resolveCapability('translation', ctx()).available).toBe(false);

    saveProvider('deepl', { apiKey: 'k-123' });
    expect(resolveCapability('translation', ctx())).toMatchObject({
      available: true,
      provider: 'deepl',
    });

    setProviderEnabled('deepl', false);
    expect(resolveCapability('translation', ctx()).available).toBe(false);
  });

  it('uses LanguageTool’s default URL, so enabling it is enough', () => {
    setProviderEnabled('languagetool', true);
    expect(resolveCapability('grammar', ctx())).toMatchObject({
      available: true,
      provider: 'languagetool',
    });
  });

  it('keeps one translation provider on at a time', () => {
    saveProvider('deepl', { apiKey: 'k' });
    saveProvider('libretranslate', { url: 'https://lt.example.org' });
    setProviderEnabled('deepl', true);
    setProviderEnabled('libretranslate', true);
    expect(getIntegrations().providers.deepl.enabled).toBe(false);
    expect(resolveCapability('translation', ctx()).provider).toBe('libretranslate');
  });

  it('does not count a provider without its required URL', () => {
    saveProvider('libretranslate', { url: '' });
    setProviderEnabled('libretranslate', true);
    expect(resolveCapability('translation', ctx()).available).toBe(false);
  });
});

describe('useCapability', () => {
  it('re-renders when integration settings change', () => {
    const { result } = renderHook(() => useCapability('enrichment'));
    expect(result.current.available).toBe(false);
    act(() => {
      saveProvider('pdl', { apiKey: 'pdl-key' });
      setProviderEnabled('pdl', true);
    });
    expect(result.current).toMatchObject({ available: true, provider: 'pdl' });
  });
});
