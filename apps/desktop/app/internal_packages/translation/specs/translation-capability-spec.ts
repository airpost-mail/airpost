import { CapabilityStore, Integrations } from 'mailspring-exports';
import {
  isTranslationAvailable,
  requestTranslation,
  translationDestination,
  translationUnavailableMessage,
} from '../lib/service';

describe('Translation and the capability store', () => {
  afterEach(() => {
    Integrations.setTransport(null);
    Integrations.keySource = null;
  });

  it('is unavailable, with the store reason, until a provider is set up', () => {
    expect(isTranslationAvailable()).toBe(false);
    expect(translationUnavailableMessage()).toBe(
      'Translation: Set up a provider in Preferences > Integrations'
    );
  });

  it('follows the store', () => {
    spyOn(CapabilityStore, 'isAvailable').andCallFake((feature) => feature === 'translation');
    expect(isTranslationAvailable()).toBe(true);
  });

  it('translates through integrations.translate as HTML', async () => {
    AppEnv.config.set('core.integrations.translation', {
      provider: 'libretranslate',
      enabled: true,
      url: 'https://lt.example.test',
    });
    Integrations.keySource = { getPassword: async () => undefined };
    const sent: any[] = [];
    Integrations.setTransport({
      available: true,
      send: async (method, params) => {
        sent.push({ method, params });
        return { translatedText: '<b>Hallo</b>', detectedLanguage: { language: 'en' } };
      },
    });
    expect(await requestTranslation('<b>Hello</b>', 'de')).toBe('<b>Hallo</b>');
    expect(sent).toEqual([
      {
        method: 'integrations.translate',
        params: {
          text: '<b>Hello</b>',
          target: 'de',
          format: 'html',
          config: { provider: 'libretranslate', url: 'https://lt.example.test' },
        },
      },
    ]);
    expect(translationDestination()).toBe('lt.example.test');
  });

  it('turns an engine error into a short message', async () => {
    AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });
    Integrations.keySource = { getPassword: async () => 'synthetic' };
    Integrations.setTransport({
      available: true,
      send: async () => {
        throw { key: 'quota-exceeded', message: 'quota (HTTP 456)', retryable: false, status: 456 };
      },
    });
    let message = '';
    try {
      await requestTranslation('<b>Hello</b>', 'de');
    } catch (err) {
      message = err.message;
    }
    expect(message).toBe("Your DeepL account's quota is used up.");
  });

  it('says it waits for the engine while the transport is the stub', async () => {
    AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });
    let message = '';
    try {
      await requestTranslation('<b>Hello</b>', 'de');
    } catch (err) {
      message = err.message;
    }
    expect(message).toBe('Waiting for engine support.');
  });
});
