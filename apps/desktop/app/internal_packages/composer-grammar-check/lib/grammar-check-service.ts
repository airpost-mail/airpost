import { CapabilityStore, Integrations } from 'mailspring-exports';

export interface GrammarError {
  offset: number;
  length: number;
  message: string;
  shortMessage: string;
  replacements: string[];
  ruleId: string;
  ruleDescription: string;
  category: string;
  categoryName: string;
  issueType: string;
  sentence: string;
  contextText: string;
  contextOffset: number;
  contextLength: number;
}

export interface GrammarCheckBackend {
  check(text: string, messageId: string, language?: string): Promise<GrammarError[]>;
  isAvailable(): boolean;
  name: string;
}

export class LanguageToolBackend implements GrammarCheckBackend {
  name = 'LanguageTool';
  private language: string;
  private disabledRules: string[] = [];

  constructor() {
    this._readConfig();
  }

  private _readConfig() {
    this.language = (AppEnv.config.get('core.composing.grammarCheckLanguage') as string) || 'auto';

    const disabledRulesStr =
      (AppEnv.config.get('core.composing.grammarCheckDisabledRules') as string) || '';
    this.disabledRules = disabledRulesStr
      ? disabledRulesStr
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean)
      : [];
  }

  refreshConfig() {
    this._readConfig();
  }

  addDisabledRule(ruleId: string) {
    if (!this.disabledRules.includes(ruleId)) {
      this.disabledRules.push(ruleId);
      AppEnv.config.set('core.composing.grammarCheckDisabledRules', this.disabledRules.join(','));
    }
  }

  isAvailable(): boolean {
    return CapabilityStore.isAvailable('grammar-check');
  }

  // The engine calls LanguageTool (crates/ms-integrations) and returns matches already in
  // GrammarError's shape, misspellings dropped: the composer's spellchecker marks those.
  async check(text: string, messageId: string, language?: string): Promise<GrammarError[]> {
    this._readConfig();
    const lang = language || this.language;
    try {
      const result = await Integrations.request('integrations.grammar', {
        text,
        language: lang && lang !== 'auto' ? lang : undefined,
        disabledRules: this.disabledRules,
      });
      return result.matches;
    } catch (err) {
      throw new Error(Integrations.errorMessage(err, 'LanguageTool'));
    }
  }
}
