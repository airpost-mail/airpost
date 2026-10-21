import { CapabilityStore, Integrations } from 'mailspring-exports';
import { LanguageToolBackend } from '../lib/service';

const match = {
  offset: 0,
  length: 4,
  message: 'Possible typo',
  shortMessage: '',
  replacements: ['This'],
  ruleId: 'EN_A_VS_AN',
  ruleDescription: 'a vs an',
  category: 'GRAMMAR',
  categoryName: 'Grammar',
  issueType: 'grammar',
  sentence: 'Thsi is it.',
  contextText: 'Thsi is it.',
  contextOffset: 0,
  contextLength: 4,
};

describe('LanguageToolBackend and the capability store', () => {
  afterEach(() => {
    Integrations.setTransport(null);
    Integrations.keySource = null;
  });

  it('is unavailable until LanguageTool is set up and the engine can call it', () => {
    expect(new LanguageToolBackend().isAvailable()).toBe(false);
    expect(CapabilityStore.unavailableMessage('grammar-check')).toBe(
      'Grammar check: Set up a provider in Preferences > Integrations'
    );
    spyOn(CapabilityStore, 'isAvailable').andCallFake((feature) => feature === 'grammar-check');
    expect(new LanguageToolBackend().isAvailable()).toBe(true);
  });

  it('checks through integrations.grammar and returns the matches as is', async () => {
    AppEnv.config.set('core.integrations.grammar', { enabled: true });
    AppEnv.config.set('core.composing.grammarCheckDisabledRules', 'WHITESPACE_RULE, COMMA');
    Integrations.keySource = { getPassword: async () => undefined };
    const sent: any[] = [];
    Integrations.setTransport({
      available: true,
      send: async (method, params) => {
        sent.push({ method, params });
        return { matches: [match] };
      },
    });

    const errors = await new LanguageToolBackend().check('Thsi is it.', 'draft-1');
    expect(errors).toEqual([match]);
    expect(sent).toEqual([
      {
        method: 'integrations.grammar',
        params: {
          text: 'Thsi is it.',
          language: undefined,
          disabledRules: ['WHITESPACE_RULE', 'COMMA'],
          config: { provider: 'languagetool' },
        },
      },
    ]);

    await new LanguageToolBackend().check('Hallo', 'draft-1', 'de-DE');
    expect(sent[1].params.language).toBe('de-DE');
  });

  it('turns an engine error into a short message', async () => {
    AppEnv.config.set('core.integrations.grammar', { enabled: true });
    Integrations.keySource = { getPassword: async () => undefined };
    Integrations.setTransport({
      available: true,
      send: async () => {
        throw { key: 'offline', message: 'timeout', retryable: true };
      },
    });
    let message = '';
    try {
      await new LanguageToolBackend().check('text', 'draft-1');
    } catch (err) {
      message = (err as Error).message;
    }
    expect(message).toBe('Could not reach LanguageTool. Check the URL and your connection.');
  });
});
