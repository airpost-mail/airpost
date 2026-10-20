import {
  Integrations,
  IntegrationsTransport,
  IntegrationMethod,
  buildProviderConfig,
  errorMessage,
  keyName,
  missingSetting,
  readSettings,
  toIntegrationError,
  IntegrationErrorKey,
} from '../src/engine/integrations';

// Synthetic keys only: the keyset is a plain object here and the real KeyManager is never used.
function fakeKeys(keys: { [name: string]: string }) {
  return {
    getPassword: jasmine.createSpy('getPassword').andCallFake(async (name: string) => keys[name]),
  };
}

function fakeTransport(respond: (method: IntegrationMethod, params: any) => any) {
  const calls: Array<{ method: IntegrationMethod; params: any }> = [];
  const transport: IntegrationsTransport & { calls: typeof calls } = {
    available: true,
    calls,
    send: async (method, params) => {
      calls.push({ method, params });
      return respond(method, params);
    },
  };
  return transport;
}

async function rejection(promise: Promise<unknown>) {
  try {
    await promise;
  } catch (err) {
    return err;
  }
  throw new Error('expected a rejection');
}

describe('Integrations client', () => {
  afterEach(() => {
    Integrations.setTransport(null);
    Integrations.keySource = null;
    Integrations.secretSource = null;
  });

  describe('keys', () => {
    function keyset(keys: { [name: string]: string }) {
      return {
        getPassword: async (name: string) => keys[name],
        replacePassword: jasmine
          .createSpy('replacePassword')
          .andCallFake(async (name: string, value: string) => {
            keys[name] = value;
          }),
        deletePassword: jasmine.createSpy('deletePassword').andCallFake(async (name: string) => {
          delete keys[name];
        }),
      };
    }
    function engine(available: boolean, secrets: { [name: string]: string } = {}) {
      return {
        secrets,
        available: () => available,
        has: async (names: string[]) => {
          const result = {};
          for (const name of names) result[name] = name in secrets;
          return result;
        },
        set: async (name: string, value: string) => {
          secrets[name] = value;
        },
        delete: async (name: string) => {
          delete secrets[name];
        },
      };
    }

    it('saves to the engine and moves a keyset copy there', async () => {
      const keys = { 'integration-deepl-key': 'old' };
      Integrations.keySource = keyset(keys);
      const secrets = engine(true);
      Integrations.secretSource = secrets;

      await Integrations.saveKey('deepl', 'synthetic');
      expect(secrets.secrets).toEqual({ 'integration:deepl': 'synthetic' });
      expect(keys).toEqual({});
      expect(await Integrations.hasKey('deepl')).toBe(true);
      expect(await Integrations.hasKey('peopledatalabs')).toBe(false);

      await Integrations.deleteKey('deepl');
      expect(secrets.secrets).toEqual({});
      expect(await Integrations.hasKey('deepl')).toBe(false);
    });

    it('keeps the keyset for an engine without account setup', async () => {
      const keys = {};
      Integrations.keySource = keyset(keys);
      Integrations.secretSource = engine(false);

      await Integrations.saveKey('deepl', 'synthetic');
      expect(keys).toEqual({ 'integration-deepl-key': 'synthetic' });
      expect(await Integrations.hasKey('deepl')).toBe(true);
      await Integrations.deleteKey('deepl');
      expect(keys).toEqual({});
    });

    it('leaves the key out of a request when only the engine has it', async () => {
      AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });
      Integrations.keySource = keyset({});
      Integrations.secretSource = engine(true, { 'integration:deepl': 'synthetic' });
      const transport = fakeTransport(() => ({ translatedText: 'Hallo' }));
      Integrations.setTransport(transport);

      await Integrations.request('integrations.translate', { text: 'Hello', target: 'de' });
      expect(transport.calls[0].params.config).toEqual({ provider: 'deepl', apiKey: '' });
    });
  });

  describe('settings', () => {
    it('defaults to no translation provider and the only provider for the other kinds', () => {
      expect(readSettings('translation')).toEqual({
        provider: null,
        enabled: false,
        url: undefined,
        username: undefined,
      });
      expect(readSettings('grammar').provider).toBe('languagetool');
      expect(readSettings('enrichment').provider).toBe('peopledatalabs');
    });

    it('ignores a provider that does not belong to the kind', () => {
      AppEnv.config.set('core.integrations.translation', {
        provider: 'languagetool',
        enabled: true,
      });
      const settings = readSettings('translation');
      expect(settings.provider).toBe(null);
      expect(settings.enabled).toBe(false);
    });

    it('names keyset entries per provider', () => {
      expect(keyName('deepl')).toBe('integration-deepl-key');
      expect(keyName('peopledatalabs')).toBe('integration-peopledatalabs-key');
    });

    it('reports what is missing', () => {
      expect(missingSetting({ provider: null, enabled: true }, false)).toBe('provider');
      expect(missingSetting({ provider: 'libretranslate', enabled: true }, false)).toBe('url');
      expect(
        missingSetting({ provider: 'libretranslate', enabled: true, url: 'https://lt.test' }, false)
      ).toBe(null);
      expect(missingSetting({ provider: 'deepl', enabled: true }, false)).toBe('key');
      expect(missingSetting({ provider: 'languagetool', enabled: true }, false)).toBe(null);
      expect(
        missingSetting({ provider: 'languagetool', enabled: true, username: 'me@x.test' }, false)
      ).toBe('key');
      expect(missingSetting({ provider: 'languagetool', enabled: true }, true)).toBe('username');
      expect(missingSetting({ provider: 'peopledatalabs', enabled: true }, true)).toBe(null);
    });
  });

  describe('buildProviderConfig', () => {
    it('builds the crate configs', () => {
      expect(
        buildProviderConfig(
          { provider: 'libretranslate', enabled: true, url: ' https://lt.test/ ' },
          ''
        )
      ).toEqual({ provider: 'libretranslate', url: 'https://lt.test/' });
      expect(
        buildProviderConfig(
          { provider: 'libretranslate', enabled: true, url: 'https://lt.test' },
          'k1'
        )
      ).toEqual({ provider: 'libretranslate', url: 'https://lt.test', apiKey: 'k1' });
      expect(buildProviderConfig({ provider: 'deepl', enabled: true }, 'abc:fx')).toEqual({
        provider: 'deepl',
        apiKey: 'abc:fx',
      });
      expect(buildProviderConfig({ provider: 'languagetool', enabled: true }, null)).toEqual({
        provider: 'languagetool',
      });
      expect(
        buildProviderConfig(
          {
            provider: 'languagetool',
            enabled: true,
            username: 'me@x.test',
            url: 'https://lt.test',
          },
          'premium'
        )
      ).toEqual({
        provider: 'languagetool',
        url: 'https://lt.test',
        username: 'me@x.test',
        apiKey: 'premium',
      });
      expect(buildProviderConfig({ provider: 'peopledatalabs', enabled: true }, 'pdl')).toEqual({
        provider: 'peopledatalabs',
        apiKey: 'pdl',
      });
      expect(buildProviderConfig({ provider: null, enabled: false }, 'x')).toBe(null);
    });
  });

  describe('request', () => {
    it('adds the saved config, key included, to the method params', async () => {
      AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });
      Integrations.keySource = fakeKeys({ 'integration-deepl-key': 'synthetic:fx' });
      const transport = fakeTransport(() => ({ translatedText: '<b>Hallo</b>' }));
      Integrations.setTransport(transport);

      const result = await Integrations.request('integrations.translate', {
        text: '<b>Hello</b>',
        target: 'de',
        format: 'html',
      });
      expect(result.translatedText).toBe('<b>Hallo</b>');
      expect(transport.calls).toEqual([
        {
          method: 'integrations.translate',
          params: {
            text: '<b>Hello</b>',
            target: 'de',
            format: 'html',
            config: { provider: 'deepl', apiKey: 'synthetic:fx' },
          },
        },
      ]);
    });

    it('uses the config of the given kind for kind-less methods', async () => {
      AppEnv.config.set('core.integrations.grammar', { enabled: true, url: 'https://lt.test' });
      Integrations.keySource = fakeKeys({});
      const transport = fakeTransport(() => []);
      Integrations.setTransport(transport);

      await Integrations.request('integrations.languages', {}, { kind: 'grammar' });
      expect(transport.calls[0].params).toEqual({
        config: { provider: 'languagetool', url: 'https://lt.test' },
      });
    });

    it('sends an explicit config as is, even for a disabled integration', async () => {
      Integrations.keySource = fakeKeys({});
      const transport = fakeTransport(() => ({ host: 'api.peopledatalabs.com' }));
      Integrations.setTransport(transport);

      const report = await Integrations.request(
        'integrations.test',
        {},
        { config: { provider: 'peopledatalabs', apiKey: 'typed' } }
      );
      expect(report.host).toBe('api.peopledatalabs.com');
      expect(transport.calls[0].params.config).toEqual({
        provider: 'peopledatalabs',
        apiKey: 'typed',
      });
      expect(Integrations.keySource.getPassword).not.toHaveBeenCalled();
    });

    it('refuses a disabled integration without calling the transport', async () => {
      AppEnv.config.set('core.integrations.enrichment', { enabled: false });
      const transport = fakeTransport(() => ({}));
      Integrations.setTransport(transport);

      const err = await rejection(
        Integrations.request('integrations.enrich', { email: 'a@b.test' })
      );
      expect(toIntegrationError(err).key).toBe('not-configured');
      expect(transport.calls.length).toBe(0);
    });

    it('fails with engine-unavailable and leaves the keyset alone until the engine is wired', async () => {
      AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });
      Integrations.keySource = fakeKeys({ 'integration-deepl-key': 'synthetic' });
      expect(Integrations.isAvailable()).toBe(false);

      const err = await rejection(
        Integrations.request('integrations.translate', { text: 'x', target: 'de' })
      );
      expect(toIntegrationError(err).key).toBe('engine-unavailable');
      expect(errorMessage(err)).toBe('Waiting for engine support.');
      expect(Integrations.keySource.getPassword).not.toHaveBeenCalled();
    });

    it('maps the serialized engine error, bare or as socket error data', async () => {
      AppEnv.config.set('core.integrations.grammar', { enabled: true });
      Integrations.keySource = fakeKeys({});
      let reply: any = {
        key: 'auth',
        message: 'rejected (HTTP 403)',
        retryable: false,
        status: 403,
      };
      Integrations.setTransport(
        fakeTransport(() => {
          throw reply;
        })
      );

      let err: any = await rejection(Integrations.request('integrations.grammar', { text: 'x' }));
      expect(err.key).toBe('auth');
      expect(err.status).toBe(403);
      expect(errorMessage(err, 'LanguageTool')).toBe(
        'LanguageTool rejected the API key or account.'
      );

      reply = {
        code: -32010,
        message: 'Integration failed',
        data: { key: 'rate-limited', message: 'slow down', retryable: true, retryAfter: 30 },
      };
      err = await rejection(Integrations.request('integrations.grammar', { text: 'x' }));
      expect(err.key).toBe('rate-limited');
      expect(err.retryable).toBe(true);
      expect(errorMessage(err, 'LanguageTool')).toBe(
        'LanguageTool is limiting requests. Try again in 30 seconds.'
      );

      reply = new Error('socket closed');
      err = await rejection(Integrations.request('integrations.grammar', { text: 'x' }));
      expect(err.key).toBe('unknown');
    });
  });

  describe('errorMessage', () => {
    it('has a distinct short message for every key', () => {
      const keys: IntegrationErrorKey[] = [
        'not-configured',
        'auth',
        'rate-limited',
        'quota-exceeded',
        'offline',
        'bad-request',
        'provider-error',
        'unsupported',
        'engine-unavailable',
        'unknown',
      ];
      const messages = keys.map((key) => errorMessage({ key, message: 'x' }, 'DeepL'));
      expect(new Set(messages).size).toBe(keys.length);
      expect(errorMessage({ key: 'quota-exceeded' }, 'DeepL')).toBe(
        "Your DeepL account's quota is used up."
      );
      expect(errorMessage({ key: 'offline' })).toBe(
        'Could not reach the provider. Check the URL and your connection.'
      );
      expect(errorMessage({ key: 'auth' })).toBe('The provider rejected the API key or account.');
    });

    it('treats an unknown key as unknown', () => {
      expect(toIntegrationError({ key: 'new-kind', message: 'm' }).key).toBe('unknown');
    });
  });
});
