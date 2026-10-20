import MailRulesStore, {
  AUTO_SINCE_KEY,
  LEGACY_AUTO_SINCE_KEY,
  LEGACY_MOVED_KEY,
  LEGACY_RULES_KEY,
  RULES_KEY,
} from '../../../src/flux/stores/mail-rules-store';
import EngineEvents from '../../../src/engine/engine-events';
import { ErrorCode, Method } from '../../../src/shared/engine-protocol';

const rule = (id: string, attrs: Record<string, any> = {}) => ({
  id,
  accountId: 'a',
  name: id,
  conditionMode: 'all',
  conditions: [{ templateKey: 'subject', comparatorKey: 'contains', value: 'x' }],
  actions: [{ templateKey: 'markAsRead', value: '' }],
  ...attrs,
});

// Let the store's awaits on the spied requests run.
const drain = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

describe('MailRulesStore', function () {
  let config: Record<string, any> = {};
  let storage: Record<string, string> = {};
  let requests: Array<{ method: string; params: any }> = [];
  let answer: (method: string, params: any) => Promise<any> = null;
  const store = MailRulesStore as any;

  beforeEach(function () {
    config = {};
    storage = {};
    requests = [];
    answer = () => Promise.resolve({});
    spyOn(AppEnv.config, 'get').andCallFake((key: string) => config[key]);
    spyOn(AppEnv.config, 'set').andCallFake((key: string, value: any) => {
      config[key] = value;
    });
    spyOn(window.localStorage, 'getItem').andCallFake((key: string) =>
      key in storage ? storage[key] : null
    );
    spyOn(window.localStorage, 'setItem').andCallFake((key: string, value: string) => {
      storage[key] = value;
    });
    spyOn(EngineEvents, 'request').andCallFake((method: string, params: any) => {
      requests.push({ method, params });
      return answer(method, params);
    });
    spyOn(AppEnv, 'showErrorDialog');
    store._rules = [];
    store._reprocessing = {};
  });

  afterEach(function () {
    store._rules = [];
    store._reprocessing = {};
  });

  describe('the rules', function () {
    it('are the mailRules setting and reload when it changes', function () {
      config[RULES_KEY] = [rule('r1')];
      store._onConfigChanged();
      expect(MailRulesStore.rules().map((r) => r.id)).toEqual(['r1']);

      // The engine disabled it.
      config[RULES_KEY] = [rule('r1', { disabled: true, disabledReason: 'Error: gone' })];
      store._onConfigChanged();
      expect(MailRulesStore.disabledRules('a').map((r) => r.disabledReason)).toEqual([
        'Error: gone',
      ]);
    });

    it('are edited through the setting', function () {
      store._onAddMailRule({ accountId: 'a', name: 'First' });
      store._onAddMailRule({ accountId: 'b', name: 'Second' });
      expect(config[RULES_KEY].map((r) => r.name)).toEqual(['First', 'Second']);
      const [first, second] = config[RULES_KEY];

      store._onUpdateMailRule(first.id, { name: 'Renamed' });
      store._onReorderMailRule(second.id, 0);
      expect(config[RULES_KEY].map((r) => r.name)).toEqual(['Second', 'Renamed']);

      store._onDisableMailRule(first.id, 'Error: broken');
      expect(config[RULES_KEY][1].disabled).toBe(true);
      expect(config[RULES_KEY][1].disabledReason).toBe('Error: broken');

      store._onDeleteMailRule(second.id);
      expect(config[RULES_KEY].map((r) => r.id)).toEqual([first.id]);
      expect(MailRulesStore.rulesForAccountId('a').map((r) => r.id)).toEqual([first.id]);
    });

    it('need an account', function () {
      expect(() => store._onAddMailRule({ name: 'Nowhere' })).toThrow();
      expect(config[RULES_KEY]).toBeUndefined();
    });
  });

  describe('the move out of localStorage', function () {
    it('copies the rules and their start time once, and keeps the old keys', function () {
      storage[LEGACY_RULES_KEY] = JSON.stringify([rule('r1'), rule('r2')]);
      storage[LEGACY_AUTO_SINCE_KEY] = '1700000000000';
      store._moveLegacyRules();
      expect(config[RULES_KEY].map((r) => r.id)).toEqual(['r1', 'r2']);
      expect(config[AUTO_SINCE_KEY]).toBe(1700000000000);
      expect(storage[LEGACY_MOVED_KEY]).toBeDefined();
      expect(storage[LEGACY_RULES_KEY]).toBeDefined();

      // Deleted after the move, it isn't brought back.
      config[RULES_KEY] = [rule('r2')];
      store._moveLegacyRules();
      expect(config[RULES_KEY].map((r) => r.id)).toEqual(['r2']);
    });

    it('keeps the rules config.json already has', function () {
      config[RULES_KEY] = [rule('r1', { disabled: true }), rule('r3')];
      storage[LEGACY_RULES_KEY] = JSON.stringify([rule('r1'), rule('r2')]);
      store._moveLegacyRules();
      expect(config[RULES_KEY].map((r) => [r.id, !!r.disabled])).toEqual([
        ['r1', true],
        ['r3', false],
        ['r2', false],
      ]);
      expect(config[AUTO_SINCE_KEY]).toBeUndefined();
    });

    it('marks unreadable rules as moved', function () {
      spyOn(console, 'warn');
      storage[LEGACY_RULES_KEY] = '{not json';
      store._moveLegacyRules();
      expect(config[RULES_KEY]).toBeUndefined();
      expect(storage[LEGACY_MOVED_KEY]).toBeDefined();
    });
  });

  describe('processing the inbox', function () {
    const progress = (jobId: string, done: number, total: number, stopped = false) =>
      EngineEvents.notification(Method.RulesProgress, {
        jobId,
        accountId: 'a',
        done,
        total,
        ...(stopped ? { stopped } : {}),
      });

    beforeEach(function () {
      store._rules = [rule('r1')];
    });

    it('asks the engine and follows its progress', async function () {
      answer = () => Promise.resolve({ jobId: 'j1', total: 120 });
      store._onStartReprocessing('a');
      expect(MailRulesStore.reprocessState().a).toEqual({ jobId: null, count: 0, total: 0 });
      await drain();
      expect(requests).toEqual([{ method: Method.RulesReprocess, params: { accountId: 'a' } }]);
      expect(MailRulesStore.reprocessState().a).toEqual({ jobId: 'j1', count: 0, total: 120 });

      progress('other', 7, 9);
      expect(MailRulesStore.reprocessState().a.count).toBe(0);
      progress('j1', 50, 120);
      expect(MailRulesStore.reprocessState().a.count).toBe(50);
      progress('j1', 120, 120);
      expect(MailRulesStore.reprocessState().a).toBeUndefined();
    });

    it('takes progress that comes before the answer', async function () {
      let resolve: (v: any) => void = null;
      answer = () => new Promise((r) => (resolve = r));
      store._onStartReprocessing('a');
      await drain();
      progress('j1', 50, 120);
      expect(MailRulesStore.reprocessState().a).toEqual({ jobId: 'j1', count: 50, total: 120 });
      resolve({ jobId: 'j1', total: 120 });
      await drain();
      expect(MailRulesStore.reprocessState().a).toEqual({ jobId: 'j1', count: 50, total: 120 });
    });

    it('cancels the job when stopped', async function () {
      answer = (method) =>
        Promise.resolve(method === Method.RulesReprocess ? { jobId: 'j1', total: 120 } : {});
      store._onStartReprocessing('a');
      await drain();
      store._onStopReprocessing('a');
      expect(requests[1]).toEqual({ method: Method.RulesCancel, params: { jobId: 'j1' } });
      expect(MailRulesStore.reprocessState().a).toBeUndefined();

      // The engine's last progress says it stopped; nothing comes back.
      progress('j1', 50, 120, true);
      expect(MailRulesStore.reprocessState().a).toBeUndefined();
    });

    it('cancels a job stopped before the engine named it', async function () {
      let resolve: (v: any) => void = null;
      answer = (method) =>
        method === Method.RulesReprocess ? new Promise((r) => (resolve = r)) : Promise.resolve({});
      store._onStartReprocessing('a');
      store._onStopReprocessing('a');
      expect(requests.length).toBe(1);
      resolve({ jobId: 'j1', total: 120 });
      await drain();
      expect(requests[1]).toEqual({ method: Method.RulesCancel, params: { jobId: 'j1' } });
      expect(MailRulesStore.reprocessState().a).toBeUndefined();
    });

    it('has nothing to do with an empty inbox or without enabled rules', async function () {
      answer = () => Promise.resolve({ jobId: 'j1', total: 0 });
      store._onStartReprocessing('a');
      await drain();
      expect(MailRulesStore.reprocessState().a).toBeUndefined();

      store._rules = [rule('r1', { disabled: true })];
      store._onStartReprocessing('a');
      expect(requests.length).toBe(1);
    });

    it('explains an account without an inbox', async function () {
      answer = () =>
        Promise.reject(Object.assign(new Error('no inbox'), { code: ErrorCode.NotFound }));
      store._onStartReprocessing('a');
      await drain();
      const dialog = AppEnv.showErrorDialog as any;
      expect(dialog).toHaveBeenCalled();
      expect(dialog.calls[0].args[0]).toContain('inbox folder');
      expect(MailRulesStore.reprocessState().a).toBeUndefined();
    });

    it('forgets the jobs when the engine goes', async function () {
      answer = () => Promise.resolve({ jobId: 'j1', total: 120 });
      store._onStartReprocessing('a');
      await drain();
      EngineEvents.emit('gone');
      expect(MailRulesStore.reprocessState()).toEqual({});
    });
  });
});
