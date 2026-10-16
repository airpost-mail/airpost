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

export interface LanguageToolMatch {
  message: string;
  shortMessage: string;
  offset: number;
  length: number;
  replacements: Array<{ value: string }>;
  context: {
    text: string;
    offset: number;
    length: number;
  };
  sentence: string;
  rule: {
    id: string;
    subId?: string;
    description: string;
    issueType: string;
    category: {
      id: string;
      name: string;
    };
    urls?: Array<{ value: string }>;
  };
}

export interface LanguageToolResponse {
  software: {
    name: string;
    version: string;
    buildDate: string;
    apiVersion: number;
    status: string;
    premium: boolean;
  };
  language: {
    name: string;
    code: string;
    detectedLanguage: {
      name: string;
      code: string;
      confidence?: number;
    };
  };
  matches: LanguageToolMatch[];
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

  // There is no LanguageTool server yet, so the store never checks and the toolbar toggle
  // is disabled.
  isAvailable(): boolean {
    return false;
  }

  async check(text: string, messageId: string, language?: string): Promise<GrammarError[]> {
    this._readConfig();
    const data = await this._request(text, language || this.language);

    const filtered = data.matches.filter((match) => match.rule.issueType !== 'misspelling');

    return filtered.map((match) => ({
      offset: match.offset,
      length: match.length,
      message: match.message,
      shortMessage: match.shortMessage || '',
      replacements: match.replacements.map((r) => r.value),
      ruleId: match.rule.id,
      ruleDescription: match.rule.description,
      category: match.rule.category.id,
      categoryName: match.rule.category.name,
      issueType: match.rule.issueType,
      sentence: match.sentence,
      contextText: match.context.text,
      contextOffset: match.context.offset,
      contextLength: match.context.length,
    }));
  }

  private async _request(text: string, language: string): Promise<LanguageToolResponse> {
    throw new Error('No grammar check server is configured.');
  }
}
