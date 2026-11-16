import { beforeEach, describe, expect, it } from 'vitest';
import {
  clearSecret,
  getIntegrations,
  isProviderConfigured,
  providerParams,
  resetIntegrations,
  saveProvider,
  setProviderEnabled,
} from './store';

beforeEach(() => resetIntegrations());

describe('integrations store', () => {
  it('never writes secrets to localStorage', () => {
    saveProvider('languagetool', {
      url: 'https://lt.example.org',
      username: 'maya',
      apiKey: 'premium-secret',
    });
    setProviderEnabled('languagetool', true);
    const raw = localStorage.getItem('airpost.integrations')!;
    expect(raw).not.toContain('premium-secret');
    expect(JSON.parse(raw).languagetool).toEqual({
      enabled: true,
      values: { url: 'https://lt.example.org', username: 'maya' },
    });
    expect(getIntegrations().secrets.languagetool).toEqual(['apiKey']);
  });

  it('keeps non-secret settings across a reload and drops secrets', () => {
    saveProvider('deepl', { apiKey: 'deepl-key' });
    setProviderEnabled('deepl', true);
    expect(isProviderConfigured(getIntegrations(), 'deepl')).toBe(true);

    resetIntegrations();
    const s = getIntegrations();
    expect(s.providers.deepl.enabled).toBe(true);
    expect(s.secrets.deepl).toEqual([]);
    expect(isProviderConfigured(s, 'deepl')).toBe(false);
  });

  it('ignores secrets that somehow ended up in storage', () => {
    localStorage.setItem(
      'airpost.integrations',
      JSON.stringify({ pdl: { enabled: true, values: { apiKey: 'leaked' } } }),
    );
    resetIntegrations();
    expect(getIntegrations().providers.pdl.values).toEqual({});
  });

  it('leaves a held secret alone when saved empty, and clears it on request', () => {
    saveProvider('pdl', { apiKey: 'one' });
    saveProvider('pdl', { apiKey: '' });
    expect(providerParams('pdl')).toEqual({ provider: 'pdl', apiKey: 'one' });
    clearSecret('pdl', 'apiKey');
    expect(providerParams('pdl')).toEqual({ provider: 'pdl' });
  });

  it('overlays unsaved edits in request params', () => {
    saveProvider('libretranslate', { url: 'https://a.example', apiKey: 'k' });
    expect(providerParams('libretranslate', { url: 'https://b.example' })).toEqual({
      provider: 'libretranslate',
      url: 'https://b.example',
      apiKey: 'k',
    });
  });
});
