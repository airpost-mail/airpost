import { describe, expect, it } from 'vitest';
import { EngineFeature, MockEngine, RpcError, RpcErrorCode, type ThreadJSON } from '@/engine';
import { errorMessage, startAiCall, toAiError } from '../api';
import {
  baseUrl,
  destinationNote,
  hostOf,
  isLocal,
  isLoopback,
  keyRequired,
  readAiSettings,
  type AiProvider,
} from '../config';
import { aiAvailability } from '../hooks';
import { replyHtml } from '../reply-insert';
import { PLUGIN_ID, answered, chipCategory, correctionTask, triageOf } from '../triage';
import { worthSummarizing } from '../summary-memory';

const p = (kind: AiProvider['kind'], url?: string): AiProvider => ({
  id: kind,
  kind,
  model: 'm',
  ...(url ? { url } : {}),
});

describe('config', () => {
  it('reads core.ai.* and drops the wrong shapes', () => {
    const s = readAiSettings({
      'core.ai.providers': [
        { id: 'o', kind: 'ollama', model: 'llama', apiKey: 'never-here' },
        { id: 'o', kind: 'openai', model: 'dup' },
        { id: 'x', kind: 'unknown' },
        'junk',
      ],
      'core.ai.features': { summary: true, reply: { enabled: true, provider: 'o' }, triage: 'yes' },
      'core.ai.accounts': ['a', 3],
      'core.ai.includeAddresses': 'true',
    });
    expect(s.providers).toEqual([{ id: 'o', kind: 'ollama', model: 'llama' }]);
    expect(s.features).toEqual({
      summary: { enabled: true },
      reply: { enabled: true, provider: 'o' },
      triage: { enabled: false },
    });
    expect(s.accounts).toEqual(['a']);
    expect(s.includeAddresses).toBe(false);
  });

  it('mirrors airpost-ai base URLs, hosts and locality', () => {
    expect(baseUrl(p('anthropic', 'https://api.anthropic.com/v1/messages'))).toBe(
      'https://api.anthropic.com',
    );
    expect(baseUrl(p('ollama', 'http://box:11434/api/chat/'))).toBe('http://box:11434');
    expect(hostOf(p('openai'))).toBe('api.openai.com');
    expect(isLocal(p('openai', 'http://192.168.1.20:1234/v1'))).toBe(true);
    expect(isLoopback(p('openai', 'http://192.168.1.20:1234/v1'))).toBe(false);
    // URL turns [::ffff:192.168.1.5] into ::ffff:c0a8:105.
    expect(isLocal(p('openai', 'http://[::ffff:192.168.1.5]:1234/v1'))).toBe(true);
    expect(isLocal(p('openai', 'http://[::ffff:10.0.0.2]/v1'))).toBe(true);
    expect(isLocal(p('openai', 'http://[::ffff:8.8.8.8]/v1'))).toBe(false);
    expect(isLoopback(p('ollama'))).toBe(true);
    expect(keyRequired(p('openai'))).toBe(true);
    expect(keyRequired(p('openai', 'http://127.0.0.1:1234/v1'))).toBe(false);
    expect(keyRequired(p('anthropic'))).toBe(true);
    expect(destinationNote(p('anthropic'))).toBe('Sent to api.anthropic.com');
    expect(destinationNote(p('ollama'))).toBe("Runs on the engine's computer (Ollama)");
    expect(destinationNote(p('openai', 'http://10.0.0.2/v1'))).toBe(
      "Sent to 10.0.0.2 (the engine's network)",
    );
  });
});

describe('aiAvailability', () => {
  const values = {
    'core.ai.providers': [
      { id: 'o', kind: 'ollama', model: 'llama' },
      { id: 'c', kind: 'anthropic', model: 'claude' },
    ],
    'core.ai.features': {
      summary: { enabled: true, provider: 'o' },
      reply: { enabled: true, provider: 'c' },
      triage: { enabled: true, provider: 'o' },
    },
    'core.ai.accounts': ['a'],
  };
  const settings = readAiSettings(values);
  const features = new Set([EngineFeature.Ai, EngineFeature.AiTriage]);
  const ctx = { settings, features, capabilities: null };

  it('says why a feature is off', () => {
    expect(aiAvailability('summary', 'a', { ...ctx, features: null }).reason).toBe('engine');
    expect(aiAvailability('summary', 'b', ctx).reason).toBe('account');
    expect(aiAvailability('summary', 'a', ctx)).toMatchObject({ available: true });
    expect(aiAvailability('reply', 'a', ctx).reason).toBe('needs-engine-key');
    const off = readAiSettings({ ...values, 'core.ai.features': {} });
    expect(aiAvailability('summary', 'a', { ...ctx, settings: off }).reason).toBe('off');
  });

  it("takes the engine's word unless it leaves the judgement to the client", () => {
    const capabilities = {
      ai: {
        summary: { available: true, credentials: 'request' },
        reply: { available: true },
        triage: { available: false, reason: 'rate-limited' },
      },
    };
    const c = { ...ctx, capabilities };
    expect(aiAvailability('summary', 'a', c).available).toBe(true);
    // An engine that holds keys says so without `credentials: request`.
    expect(aiAvailability('reply', 'a', c).available).toBe(true);
    expect(aiAvailability('triage', 'a', c).reason).toBe('rate-limited');
  });
});

describe('api', () => {
  it('maps engine errors', () => {
    const e = toAiError(
      new RpcError({
        code: RpcErrorCode.AiFailed,
        message: 'AI request failed',
        data: { key: 'rate-limited', message: 'slow down', retryable: true, retryAfter: 12 },
      }),
    );
    expect(e).toMatchObject({ key: 'rate-limited', retryAfter: 12 });
    expect(errorMessage(e, 'Ollama')).toBe('Ollama is limiting requests. Try again in 12 seconds.');
    expect(toAiError(new RpcError({ code: RpcErrorCode.EngineBusy, message: 'busy' })).key).toBe(
      'engine-busy',
    );
    expect(
      toAiError(new RpcError({ code: RpcErrorCode.AiFailed, message: 'x', data: { key: 'new' } }))
        .key,
    ).toBe('unknown');
  });

  it('collects only its own stream and reports a cancel as cancelled', async () => {
    const engine = new MockEngine({ latency: 5, aiDemo: true });
    await engine.connect();
    const t = [...engine.store.threads.values()].find(
      (t) => engine.store.threadMessages(t.id).length >= 2,
    )!;
    const mine: string[] = [];
    const other = startAiCall(engine, 'ai.summarize', { accountId: t.aid, threadId: t.id });
    const call = startAiCall(
      engine,
      'ai.summarize',
      { accountId: t.aid, threadId: t.id, refresh: true },
      (text) => mine.push(text),
    );
    const r = await call.promise;
    expect(mine.join('')).toBe(r.text);
    await other.promise;

    const third = startAiCall(
      engine,
      'ai.summarize',
      { accountId: t.aid, threadId: t.id, refresh: true },
      () => third.cancel(),
    );
    await expect(third.promise).rejects.toMatchObject({ key: 'cancelled' });
  });
});

function thread(over: Partial<ThreadJSON>, value?: object): ThreadJSON {
  return {
    id: 't',
    aid: 'a',
    v: 1,
    __cls: 'Thread',
    subject: 's',
    unread: 0,
    starred: 0,
    lmt: 100,
    fmt: 50,
    lmst: 0,
    lmrt: 100,
    inAllMail: true,
    attachmentCount: 0,
    folders: [],
    labels: [],
    participants: [],
    metadata: value ? [{ pluginId: PLUGIN_ID, v: 1, value }] : [],
    ...over,
  };
}

describe('triage', () => {
  it('shows needs-reply until the user answers, and important', () => {
    const v = { category: 'needs-reply', needsReply: true };
    expect(chipCategory(triageOf(thread({}, v)), thread({}, v))).toBe('needs-reply');
    const replied = thread({ lmst: 120 }, v);
    expect(answered(triageOf(replied)!, replied)).toBe(true);
    expect(chipCategory(triageOf(replied), replied)).toBe(null);
    const filedLater = thread({ lmst: 120 }, { ...v, source: 'user', at: 130 });
    expect(chipCategory(triageOf(filedLater), filedLater)).toBe('needs-reply');
    const imp = thread({}, { category: 'important' });
    expect(chipCategory(triageOf(imp), imp)).toBe('important');
    const news = thread({}, { category: 'newsletter' });
    expect(chipCategory(triageOf(news), news)).toBe(null);
    expect(triageOf(thread({}, { category: 'made-up' }))).toBe(null);
  });

  it("files the user's category without the model's reason", () => {
    const t = thread({}, { category: 'other', reason: 'model says', model: 'm', messageId: 'x' });
    const task = correctionTask(t, 'needs-reply');
    expect(task).toMatchObject({
      __cls: 'SyncbackMetadataTask',
      aid: 'a',
      pluginId: PLUGIN_ID,
      modelId: 't',
      modelClassName: 'thread',
    });
    const value = (task as { value: Record<string, unknown> }).value;
    expect(value).toMatchObject({
      category: 'needs-reply',
      needsReply: true,
      source: 'user',
      model: 'm',
    });
    expect(value).not.toHaveProperty('reason');
  });
});

describe('helpers', () => {
  it('escapes model text for the editor', () => {
    expect(replyHtml(' <a href="x">hi</a>\nthere ')).toBe(
      '&lt;a href=&quot;x&quot;&gt;hi&lt;/a&gt;<br>there',
    );
  });

  it('offers a summary for two messages or one long one', () => {
    const m = (body: string, draft = false) => ({ draft, body }) as never;
    expect(worthSummarizing([m('a'), m('b')])).toBe(true);
    expect(worthSummarizing([m('a'), m('b', true)])).toBe(false);
    expect(worthSummarizing([m('x'.repeat(4000))])).toBe(true);
  });
});
