import BaseStore from 'mailspring-store';
import * as Utils from '../models/utils';
import * as Actions from '../actions';
import { localized } from '../../intl/localize';
import EngineEvents, { EngineNotification } from '../../engine/client/events';
import {
  ErrorCode,
  Method,
  RulesProgressParams,
  RulesReprocessResult,
} from '../../engine/protocol';

import { Template } from '../../components/scenario-editor/models';
import { ConditionMode, ConditionTemplates, ActionTemplates } from '../mail-rules-templates';

/*
The editor of the mail rules (docs/desktop-client-plan.md 4.2, T12). The rules are the
`mailRules` setting, which the main process mirrors between config.json and the engine
(src/engine/supervisor/settings.ts). The engine applies them to arriving mail, disables a rule
that fails (writing `disabled` and `disabledReason` into the setting) and runs "Process entire
inbox" as `rules.reprocess`, reporting `rules.progress` to this window's connection.

The rules lived in the main window's localStorage before; they move into config.json once.
*/

export const RULES_KEY = 'mailRules';
export const AUTO_SINCE_KEY = 'mailRulesAutoSince';

// Where the rules lived before the engine ran them.
export const LEGACY_RULES_KEY = 'MailRules-V2';
export const LEGACY_AUTO_SINCE_KEY = 'MailRules-Auto-Since';
// Set once the legacy rules moved; the old keys stay for a downgrade.
export const LEGACY_MOVED_KEY = 'MailRules-MovedToSettings';

export interface MailRule extends Template {
  id: string;
  accountId: string;
  disabled?: boolean;
  disabledReason?: string;
  name: string;
  conditions: [
    {
      templateKey: string;
      comparatorKey: string;
      value: string;
    },
  ];
  conditionMode: 'any' | 'all';
  actions: [
    {
      value: string;
      // Display name of the folder / label chosen for `value`, used to re-resolve the
      // category when its id changes (folder ids are hashes of the IMAP path).
      valueName?: string;
      templateKey: string;
    },
  ];
}

export interface ReprocessState {
  // Null until the engine answers rules.reprocess.
  jobId: string | null;
  // Inbox threads gone through so far, of `total`.
  count: number;
  total: number;
}

const copy = <T>(v: T): T => JSON.parse(JSON.stringify(v));

class MailRulesStore extends BaseStore {
  _reprocessing: { [accountId: string]: ReprocessState } = {};
  _rules: MailRule[] = [];

  constructor() {
    super();
    if (AppEnv.isMainWindow()) this._moveLegacyRules();
    this._rules = this._readRules();

    this.listenTo(Actions.addMailRule, this._onAddMailRule);
    this.listenTo(Actions.deleteMailRule, this._onDeleteMailRule);
    this.listenTo(Actions.reorderMailRule, this._onReorderMailRule);
    this.listenTo(Actions.updateMailRule, this._onUpdateMailRule);
    this.listenTo(Actions.disableMailRule, this._onDisableMailRule);
    this.listenTo(Actions.startReprocessingMailRules, this._onStartReprocessing);
    this.listenTo(Actions.stopReprocessingMailRules, this._onStopReprocessing);

    // The engine disables rules, and other windows and clients edit them.
    AppEnv.config.onDidChange(RULES_KEY, this._onConfigChanged);
    EngineEvents.on('notification', this._onEngineNotification);
    EngineEvents.on('gone', this._onEngineGone);
  }

  rules() {
    return this._rules;
  }

  rulesForAccountId(accountId: string) {
    return this._rules.filter((f) => f.accountId === accountId);
  }

  disabledRules(accountId?: string) {
    return this._rules.filter((f) => (!accountId || f.accountId === accountId) && f.disabled);
  }

  reprocessState() {
    return this._reprocessing;
  }

  _readRules(): MailRule[] {
    const rules = AppEnv.config.get(RULES_KEY);
    return Array.isArray(rules) ? copy(rules) : [];
  }

  _onConfigChanged = () => {
    this._rules = this._readRules();
    this.trigger();
  };

  /*
  The one-time move of the rules, and of the time they apply from, out of localStorage. Rules
  config.json already has (from the engine, another client or an earlier move) stay; a legacy
  rule with the same id isn't added again.
  */
  _moveLegacyRules() {
    const storage = window.localStorage;
    if (storage.getItem(LEGACY_MOVED_KEY)) return;
    let legacy: MailRule[] = [];
    try {
      const txt = storage.getItem(LEGACY_RULES_KEY);
      legacy = txt ? JSON.parse(txt) : [];
    } catch (err) {
      console.warn('Could not read the saved mail rules', err);
    }
    // The time first, so the engine never runs the moved rules with a later one.
    const since = Number(storage.getItem(LEGACY_AUTO_SINCE_KEY) || 0);
    if (since > 0) AppEnv.config.set(AUTO_SINCE_KEY, since);
    if (Array.isArray(legacy) && legacy.length) {
      const current = this._readRules();
      const ids = new Set(current.map((r) => r.id));
      AppEnv.config.set(RULES_KEY, [...current, ...legacy.filter((r) => r && !ids.has(r.id))]);
    }
    storage.setItem(LEGACY_MOVED_KEY, `${Date.now()}`);
  }

  _onDeleteMailRule = (id: string) => {
    this._save(this._rules.filter((f) => f.id !== id));
  };

  _onReorderMailRule = (id: string, newIdx: number) => {
    const rules = [...this._rules];
    const currentIdx = rules.findIndex((r) => r.id === id);
    if (currentIdx === -1) {
      return;
    }
    const [rule] = rules.splice(currentIdx, 1);
    rules.splice(newIdx, 0, rule);
    this._save(rules);
  };

  _onAddMailRule = (properties: Partial<MailRule> & { accountId: string }) => {
    const defaults = {
      id: Utils.generateTempId(),
      name: localized('Untitled Rule'),
      conditionMode: ConditionMode.All,
      conditions: [ConditionTemplates[0].createDefaultInstance()],
      actions: [ActionTemplates[0].createDefaultInstance()],
      disabled: false,
    };

    if (!properties.accountId) {
      throw new Error('AddMailRule: you must provide an account id.');
    }
    this._save([...this._rules, Object.assign(defaults, properties) as MailRule]);
  };

  _onUpdateMailRule = (id: string, properties: Partial<MailRule>) => {
    if (!this._rules.some((f) => f.id === id)) {
      return;
    }
    this._save(this._rules.map((r) => (r.id === id ? ({ ...r, ...properties } as MailRule) : r)));
  };

  _onDisableMailRule = (id: string, reason: string) => {
    const existing = this._rules.find((f) => id === f.id);
    if (!existing || existing.disabled === true) {
      return;
    }
    this._save(
      this._rules.map((r) =>
        r.id === id ? ({ ...r, disabled: true, disabledReason: reason } as MailRule) : r
      )
    );
  };

  _save(rules: MailRule[]) {
    this._rules = rules;
    AppEnv.config.set(RULES_KEY, rules);
    this.trigger();
  }

  // Reprocessing Existing Mail

  _onStartReprocessing = async (aid: string) => {
    // The preferences UI explains this to the user before dispatching; here we only
    // avoid walking the entire inbox for nothing.
    if (!this._rules.some((r) => r.accountId === aid && !r.disabled) || this._reprocessing[aid]) {
      return;
    }
    this._reprocessing[aid] = { jobId: null, count: 0, total: 0 };
    this.trigger();
    try {
      const { jobId, total } = await EngineEvents.request<RulesReprocessResult>(
        Method.RulesReprocess,
        { accountId: aid }
      );
      const state = this._reprocessing[aid];
      if (!state || total === 0) {
        // Stopped before the engine answered, finished already, or nothing to go through.
        if (!state) EngineEvents.request(Method.RulesCancel, { jobId }).catch(() => {});
        delete this._reprocessing[aid];
      } else {
        this._reprocessing[aid] = { ...state, jobId, total };
      }
    } catch (err) {
      delete this._reprocessing[aid];
      AppEnv.showErrorDialog(
        err && err.code === ErrorCode.NotFound
          ? localized(
              `Sorry, this account does not appear to have an inbox folder so this feature is disabled.`
            )
          : err.message
      );
    }
    this.trigger();
  };

  _onEngineNotification = ({ method, params }: EngineNotification) => {
    if (method !== Method.RulesProgress || !params) return;
    const p = params as RulesProgressParams;
    const state = this._reprocessing[p.accountId];
    // The first progress can come before the answer that names the job.
    if (!state || (state.jobId && state.jobId !== p.jobId)) return;
    if (p.stopped || p.done >= p.total) {
      delete this._reprocessing[p.accountId];
    } else {
      this._reprocessing[p.accountId] = { jobId: p.jobId, count: p.done, total: p.total };
    }
    this.trigger();
  };

  // No more progress comes over a dropped connection.
  _onEngineGone = () => {
    if (!Object.keys(this._reprocessing).length) return;
    this._reprocessing = {};
    this.trigger();
  };

  _onStopReprocessing = (aid: string) => {
    const state = this._reprocessing[aid];
    if (!state) return;
    if (state.jobId) {
      EngineEvents.request(Method.RulesCancel, { jobId: state.jobId }).catch(() => {});
    }
    delete this._reprocessing[aid];
    this.trigger();
  };
}

export default new MailRulesStore();
