import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';
import { KeyManager, Integrations } from 'mailspring-exports';

import {
  IntegrationSection,
  privacyNote,
} from '../../../src/features/preferences/tabs/integrations';

const SYNTHETIC_KEY = 'synthetic-test-key-0000:fx';

// The keyset is a plain object; the real KeyManager methods are spied out so no keychain item
// is read or written.
function fakeKeyset() {
  const keys: { [name: string]: string } = {};
  spyOn(KeyManager, 'getPassword').andCallFake(async (name: string) => keys[name]);
  spyOn(KeyManager, 'replacePassword').andCallFake(async (name: string, value: string) => {
    keys[name] = value;
  });
  spyOn(KeyManager, 'deletePassword').andCallFake(async (name: string) => {
    delete keys[name];
  });
  return keys;
}

async function render(kind: 'translation' | 'grammar' | 'enrichment') {
  let renderer: TestRenderer.ReactTestRenderer;
  await act(async () => {
    renderer = TestRenderer.create(<IntegrationSection kind={kind} />);
  });
  return renderer;
}

function byId(renderer: TestRenderer.ReactTestRenderer, id: string) {
  return renderer.root.find((node) => node.props.id === id);
}

async function type(renderer: TestRenderer.ReactTestRenderer, id: string, value: string) {
  await act(async () => {
    byId(renderer, id).props.onChange({ target: { value } });
  });
  await act(async () => {
    await byId(renderer, id).props.onBlur?.();
  });
}

/** Types a key and presses Enter, the field's only save paths besides Save Key. */
async function typeKey(renderer: TestRenderer.ReactTestRenderer, id: string, value: string) {
  await type(renderer, id, value);
  await act(async () => {
    await byId(renderer, id).props.onKeyDown({ key: 'Enter' });
  });
}

function button(renderer: TestRenderer.ReactTestRenderer, label: string) {
  return renderer.root.find((node) => node.type === 'button' && node.props.children === label);
}

/** Opens a card that is off, as Set up does; its fields stay hidden until then. */
async function setUp(renderer: TestRenderer.ReactTestRenderer) {
  await act(async () => {
    button(renderer, 'Set up').props.onClick();
  });
}

function section(renderer: TestRenderer.ReactTestRenderer): IntegrationSection {
  return renderer.root.findByType(IntegrationSection).instance;
}

function textOf(renderer: TestRenderer.ReactTestRenderer) {
  return JSON.stringify(renderer.toJSON());
}

describe('Preferences > Integrations', () => {
  let keys: { [name: string]: string };

  beforeEach(() => {
    keys = fakeKeyset();
  });

  afterEach(() => {
    Integrations.setTransport(null);
  });

  it('saves the DeepL key to the keyset and never to config', async () => {
    const renderer = await render('translation');
    await setUp(renderer);
    await act(async () => {
      byId(renderer, 'integrations-translation-provider').props.onChange({
        target: { value: 'deepl' },
      });
    });
    await act(async () => {
      button(renderer, 'Turn on').props.onClick();
    });
    await typeKey(renderer, 'integrations-translation-key', SYNTHETIC_KEY);

    expect(KeyManager.replacePassword).toHaveBeenCalledWith('integration-deepl-key', SYNTHETIC_KEY);
    expect(keys['integration-deepl-key']).toBe(SYNTHETIC_KEY);
    expect(AppEnv.config.get('core.integrations.translation')).toEqual({
      provider: 'deepl',
      enabled: true,
    });
    expect(JSON.stringify(AppEnv.config.get('core'))).not.toContain(SYNTHETIC_KEY);
    // The saved key isn't shown again.
    expect(byId(renderer, 'integrations-translation-key').props.value).toBe('');
    expect(textOf(renderer)).not.toContain(SYNTHETIC_KEY);
    renderer.unmount();
  });

  it('keeps the LanguageTool URL and username in config and the API key in the keyset', async () => {
    const renderer = await render('grammar');
    await setUp(renderer);
    await type(renderer, 'integrations-grammar-url', 'https://lt.example.test');
    await type(renderer, 'integrations-grammar-username', 'me@example.test');
    await typeKey(renderer, 'integrations-grammar-key', SYNTHETIC_KEY);

    expect(AppEnv.config.get('core.integrations.grammar')).toEqual({
      provider: 'languagetool',
      enabled: false,
      url: 'https://lt.example.test',
      username: 'me@example.test',
    });
    expect(keys['integration-languagetool-key']).toBe(SYNTHETIC_KEY);
    expect(JSON.stringify(AppEnv.config.get('core'))).not.toContain(SYNTHETIC_KEY);
    renderer.unmount();
  });

  it('shows that the engine is not ready when testing, without reading the key', async () => {
    keys['integration-peopledatalabs-key'] = SYNTHETIC_KEY;
    AppEnv.config.set('core.integrations.enrichment', { enabled: true });
    const renderer = await render('enrichment');
    (KeyManager.getPassword as any).reset();

    const test = renderer.root.find(
      (node) => node.type === 'button' && node.props.children === 'Test'
    );
    await act(async () => {
      await test.props.onClick();
    });
    expect(textOf(renderer)).toContain('Waiting for engine support.');
    expect(KeyManager.getPassword).not.toHaveBeenCalled();
    renderer.unmount();
  });

  it('tests the typed key over the saved one once the engine is there', async () => {
    keys['integration-deepl-key'] = 'saved-synthetic';
    AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });
    const sent: any[] = [];
    Integrations.setTransport({
      available: true,
      send: async (method, params) => {
        sent.push({ method, params });
        return { host: 'api-free.deepl.com', usage: { used: 1200, limit: 500000 } };
      },
    });
    const renderer = await render('translation');
    await act(async () => {
      byId(renderer, 'integrations-translation-key').props.onChange({
        target: { value: 'typed-synthetic:fx' },
      });
    });
    await act(async () => {
      await section(renderer)._onTest();
    });
    expect(sent).toEqual([
      {
        method: 'integrations.test',
        params: { config: { provider: 'deepl', apiKey: 'typed-synthetic:fx' } },
      },
    ]);
    expect(textOf(renderer)).toContain('Connected to api-free.deepl.com.');
    renderer.unmount();
  });

  it('keeps the key when turned off and deletes it on Remove', async () => {
    keys['integration-peopledatalabs-key'] = SYNTHETIC_KEY;
    AppEnv.config.set('core.integrations.enrichment', { enabled: true });
    const renderer = await render('enrichment');
    await act(async () => {
      button(renderer, 'Turn off').props.onClick();
    });
    expect(AppEnv.config.get('core.integrations.enrichment').enabled).toBe(false);
    expect(keys['integration-peopledatalabs-key']).toBe(SYNTHETIC_KEY);

    await act(async () => {
      await section(renderer)._onRemove();
    });
    expect(KeyManager.deletePassword).toHaveBeenCalledWith('integration-peopledatalabs-key');
    expect(keys['integration-peopledatalabs-key']).toBeUndefined();
    renderer.unmount();
  });

  it('does not save a typed key on blur, only on Save Key', async () => {
    AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });
    const renderer = await render('translation');
    await type(renderer, 'integrations-translation-key', SYNTHETIC_KEY);
    expect(KeyManager.replacePassword).not.toHaveBeenCalled();
    expect(byId(renderer, 'integrations-translation-key').props.value).toBe(SYNTHETIC_KEY);

    await act(async () => {
      await button(renderer, 'Save key').props.onClick();
    });
    expect(keys['integration-deepl-key']).toBe(SYNTHETIC_KEY);
    expect(byId(renderer, 'integrations-translation-key').props.value).toBe('');
    renderer.unmount();
  });

  it('shows a slow Test result after the key field blurs, and after Enter saves the key', async () => {
    AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });
    let reply: (value: any) => void;
    Integrations.setTransport({
      available: true,
      send: () => new Promise((resolve) => (reply = resolve)),
    });
    const renderer = await render('translation');
    // The click path: mousedown blurs the field, then Test runs.
    await type(renderer, 'integrations-translation-key', 'typed-synthetic:fx');
    await act(async () => {
      button(renderer, 'Test').props.onClick();
    });
    expect(keys['integration-deepl-key']).toBeUndefined();
    // Saving the same key while the request is out keeps its result.
    await act(async () => {
      await byId(renderer, 'integrations-translation-key').props.onKeyDown({ key: 'Enter' });
    });
    expect(keys['integration-deepl-key']).toBe('typed-synthetic:fx');
    await act(async () => {
      reply({ host: 'api-free.deepl.com' });
    });
    expect(textOf(renderer)).toContain('Connected to api-free.deepl.com.');
    renderer.unmount();
  });

  it('drops a Test result once the key is edited while the request is out', async () => {
    AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });
    let reply: (value: any) => void;
    Integrations.setTransport({
      available: true,
      send: () => new Promise((resolve) => (reply = resolve)),
    });
    const renderer = await render('translation');
    await type(renderer, 'integrations-translation-key', 'first-synthetic:fx');
    await act(async () => {
      button(renderer, 'Test').props.onClick();
    });
    await type(renderer, 'integrations-translation-key', 'second-synthetic:fx');
    await act(async () => {
      reply({ host: 'api-free.deepl.com' });
    });
    expect(textOf(renderer)).not.toContain('Connected to');
    expect(button(renderer, 'Test').props.disabled).toBe(false);
    renderer.unmount();
  });

  it('deletes the keys of every translation provider on Remove', async () => {
    keys['integration-libretranslate-key'] = 'libre-synthetic';
    keys['integration-deepl-key'] = SYNTHETIC_KEY;
    AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });
    const renderer = await render('translation');
    await act(async () => {
      await button(renderer, 'Remove').props.onClick();
    });
    expect(keys['integration-deepl-key']).toBeUndefined();
    expect(keys['integration-libretranslate-key']).toBeUndefined();
    renderer.unmount();
  });

  it('offers to remove keys left behind when no provider is chosen', async () => {
    keys['integration-libretranslate-key'] = 'libre-synthetic';
    AppEnv.config.set('core.integrations.translation', { provider: 'libretranslate' });
    const renderer = await render('translation');
    await setUp(renderer);
    await act(async () => {
      byId(renderer, 'integrations-translation-provider').props.onChange({ target: { value: '' } });
    });
    await act(async () => {
      await button(renderer, 'Remove saved keys').props.onClick();
    });
    expect(keys['integration-libretranslate-key']).toBeUndefined();
    expect(textOf(renderer)).not.toContain('Remove saved keys');
    renderer.unmount();
  });

  it('keeps the fields of an integration that is off hidden until Set up', async () => {
    const renderer = await render('grammar');
    expect(renderer.root.findAll((n) => n.props.id === 'integrations-grammar-url').length).toBe(0);
    expect(textOf(renderer)).toContain('Off');
    await setUp(renderer);
    expect(byId(renderer, 'integrations-grammar-url')).toBeDefined();
    await act(async () => {
      button(renderer, 'Turn on').props.onClick();
    });
    expect(AppEnv.config.get('core.integrations.grammar').enabled).toBe(true);
    expect(textOf(renderer)).toContain('Turn off');
    renderer.unmount();
  });

  it('names the premium LanguageTool host for premium credentials on the default URL', () => {
    const premium = {
      provider: 'languagetool' as const,
      enabled: true,
      username: 'me@example.test',
    };
    expect(privacyNote('grammar', premium, true)).toContain('api.languagetoolplus.com');
    expect(
      privacyNote('grammar', { ...premium, url: 'https://api.languagetool.org/v2/' }, true)
    ).toContain('api.languagetoolplus.com');
    // Free tier: no key, or no username.
    expect(privacyNote('grammar', premium, false)).toContain('api.languagetool.org');
    expect(privacyNote('grammar', { ...premium, username: '' }, true)).toContain(
      'api.languagetool.org'
    );
    // A self-hosted server is used as given.
    expect(privacyNote('grammar', { ...premium, url: 'https://lt.example.test' }, true)).toContain(
      'lt.example.test'
    );
  });
});
