// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import React from 'react';
import {
  AI,
  AccountStore,
  CapabilityStore,
  localized,
  AiFeature,
  AiModelInfo,
  AiProvider,
  AiProviderKind,
  AiSettings,
  AiUsageRow,
  Engine,
} from 'mailspring-exports';
import {
  SettingsPage,
  SettingsSection,
  SettingsCard,
  SettingsNote,
  SettingsRow,
  SettingsSwitch,
  SettingsField,
  StatusPill,
  StatusKind,
  ProviderCard,
  ProviderLogo,
  PrivacyNote,
} from 'mailspring-component-kit';
import {
  errorMessage,
  forgetKey,
  hasSavedKey,
  isCancelled,
  listModels,
  saveKey,
  testProvider,
} from './ai-client';
import { TRIAGE_ENGINE_FEATURE, TriageRun, triageInbox } from './triage';

// Preferences > AI (docs/ai-plan.md section 7). Settings go to config under core.ai; API keys to
// the engine's secret store (integration:ai-<id>), or the keyset (integration-ai-<id>-key) with
// an engine that can't keep them, and are never shown again once saved.

const KIND_ORDER: AiProviderKind[] = ['anthropic', 'openai', 'ollama'];

const LOGO_COLORS: { [kind in AiProviderKind]: string } = {
  anthropic: '#c96442',
  openai: '#10a37f',
  ollama: '#3a3d45',
};

const FEATURES: {
  feature: AiFeature;
  label: () => string;
  description: () => string;
  capability: 'ai-summary' | 'ai-reply' | 'ai-triage';
}[] = [
  {
    feature: 'summary',
    label: () => localized('Thread summaries'),
    description: () => localized('A Summarize button above long threads.'),
    capability: 'ai-summary',
  },
  {
    feature: 'reply',
    label: () => localized('Reply suggestions'),
    description: () => localized('Suggested replies, or a draft written from a few words.'),
    capability: 'ai-reply',
  },
  {
    feature: 'triage',
    label: () => localized('Inbox sorting'),
    description: () =>
      localized('Tags new inbox mail as Needs reply, Important, Newsletter and so on.'),
    capability: 'ai-triage',
  },
];

function urlPlaceholder(kind: AiProviderKind) {
  if (kind === 'openai') return `${AI.KINDS.openai.defaultUrl} or http://localhost:1234/v1`;
  return AI.KINDS[kind].defaultUrl;
}

/** Scheme, host and port of the provider's base URL: where its key is sent. */
function serverOf(provider: AiProvider) {
  try {
    return new URL(AI.baseUrl(provider)).origin;
  } catch (err) {
    return '';
  }
}

/** Where the provider runs, in words: "On this computer", a host on the network, or a host. */
function placeOf(provider: AiProvider) {
  if (AI.isLoopback(provider)) return localized('On this computer');
  const host = AI.hostOf(provider) || AI.KINDS[provider.kind].name;
  return AI.isLocal(provider) ? localized('%@ on your network', host) : host;
}

function PrivacyFor({ provider }: { provider: AiProvider }) {
  if (AI.isLoopback(provider)) {
    return <PrivacyNote kind="local">{localized('Mail stays on this computer.')}</PrivacyNote>;
  }
  const host = AI.hostOf(provider) || AI.providerName(provider);
  const where = AI.isLocal(provider)
    ? localized(
        'The parts of mail listed under "What is sent to the model" go to %@ on your network when you use a feature.',
        host
      )
    : localized(
        'The parts of mail listed under "What is sent to the model" go to %@ when you use a feature.',
        host
      );
  return (
    <PrivacyNote kind="network">
      {where}
      {provider.kind !== 'ollama' &&
        ` ${localized('The key is stored encrypted, like your passwords.')}`}
    </PrivacyNote>
  );
}

interface ProviderProps {
  provider: AiProvider;
  onChange: (provider: AiProvider) => void;
  onRemove: () => Promise<void> | void;
}

interface ProviderState {
  draft: AiProvider;
  open: boolean;
  keyInput: string;
  hasKey: boolean;
  /** Whether hasKey was read; unknown while the engine is not connected. */
  keyKnown: boolean;
  /** Why the saved key went away, shown until a key is saved again. */
  keyNotice: string;
  models: AiModelInfo[];
  loadingModels: boolean;
  modelsError: string;
  testing: boolean;
  testResult: { ok: boolean; message: string } | null;
}

export class ProviderSection extends React.Component<ProviderProps, ProviderState> {
  static displayName = 'AiProviderSection';

  _mounted = false;
  /** Bumped whenever the form changes, so a reply for an older form is dropped. */
  _formGeneration = 0;

  constructor(props: ProviderProps) {
    super(props);
    this.state = {
      draft: props.provider,
      // A provider just added has no model yet and opens to be set up.
      open: !props.provider.model,
      keyInput: '',
      hasKey: false,
      keyKnown: false,
      keyNotice: '',
      models: [],
      loadingModels: false,
      modelsError: '',
      testing: false,
      testResult: null,
    };
  }

  componentDidMount() {
    this._mounted = true;
    this._loadKeyPresence();
    // Where the key is saved depends on the engine connected.
    Engine.events.on('connected', this._loadKeyPresence);
  }

  componentDidUpdate(prev: ProviderProps) {
    if (JSON.stringify(prev.provider) !== JSON.stringify(this.props.provider)) {
      this.setState({ draft: { ...this.props.provider } });
    }
  }

  componentWillUnmount() {
    this._mounted = false;
    Engine.events.removeListener('connected', this._loadKeyPresence);
  }

  _id(field: string) {
    return `ai-provider-${this.props.provider.id}-${field}`;
  }

  _loadKeyPresence = async () => {
    let hasKey = false;
    let keyKnown = true;
    try {
      hasKey = await hasSavedKey(this.props.provider.id);
    } catch (err) {
      // Unknown: offer Save key, not Forget key.
      keyKnown = false;
    }
    if (this._mounted) this.setState({ hasKey, keyKnown });
  };

  _keyFailed(err: any) {
    if (!this._mounted) return;
    this.setState({
      keyNotice: localized('Could not change the saved API key: %@', (err && err.message) || err),
    });
  }

  _onChangeField =
    (field: 'name' | 'url' | 'model' | 'contextTokens') =>
    (event: React.ChangeEvent<HTMLInputElement>) => {
      this._formGeneration++;
      const raw = event.target.value;
      const value = field === 'contextTokens' ? (raw ? Number(raw) : undefined) : raw;
      this.setState({ draft: { ...this.state.draft, [field]: value }, testResult: null });
    };

  _onCommit = async () => {
    const { draft } = this.state;
    const previous = this.props.provider;
    const next: AiProvider = { ...draft, model: (draft.model || '').trim() };
    if (JSON.stringify(next) === JSON.stringify(previous)) return;
    // The saved key was given for the old server; it must not go to a new one.
    if (serverOf(next) !== serverOf(previous)) {
      let saved = true;
      try {
        saved = await hasSavedKey(previous.id);
      } catch (err) {
        // Unknown: remove it anyway.
      }
      if (saved) {
        try {
          await forgetKey(previous.id);
        } catch (err) {
          // The engine is unreachable: keep the old server, whose key it may still hold.
          this._keyFailed(err);
          return;
        }
        if (this._mounted) {
          this.setState({
            hasKey: false,
            keyNotice: localized('The server changed, so the saved API key was removed.'),
          });
        }
      }
    }
    this.props.onChange(next);
  };

  _onPickModel = (event: React.ChangeEvent<HTMLSelectElement>) => {
    if (!event.target.value) return;
    this._formGeneration++;
    const draft = { ...this.state.draft, model: event.target.value };
    this.setState({ draft, testResult: null }, this._onCommit);
  };

  _onChangeKey = (event: React.ChangeEvent<HTMLInputElement>) => {
    this._formGeneration++;
    this.setState({ keyInput: event.target.value, testResult: null });
  };

  /** Saves the typed key on Save key or Enter only, never on blur: Test must not store a key
   * before it was tested. */
  _onSaveKey = async () => {
    const value = this.state.keyInput.trim();
    if (!value) return;
    try {
      await saveKey(this.props.provider.id, value);
    } catch (err) {
      this._keyFailed(err);
      return;
    }
    if (this._mounted && this.state.keyInput.trim() === value) {
      this.setState({ keyInput: '', hasKey: true, keyKnown: true, keyNotice: '' });
    }
  };

  _onForgetKey = async () => {
    this._formGeneration++;
    try {
      await forgetKey(this.props.provider.id);
    } catch (err) {
      this._keyFailed(err);
      return;
    }
    if (this._mounted) {
      this.setState({ keyInput: '', hasKey: false, testResult: null, keyNotice: '' });
    }
  };

  _onLoadModels = async () => {
    const generation = this._formGeneration;
    this.setState({ loadingModels: true, modelsError: '' });
    let models: AiModelInfo[] = [];
    let modelsError = '';
    try {
      models = await listModels(this.state.draft, this.state.keyInput);
    } catch (err) {
      modelsError = errorMessage(err, AI.providerName(this.state.draft));
    }
    if (!this._mounted) return;
    if (generation !== this._formGeneration) {
      this.setState({ loadingModels: false });
      return;
    }
    this.setState({ loadingModels: false, models, modelsError });
  };

  /** Tests what the form shows, saved or not: the typed key wins over the saved one. */
  _onTest = async () => {
    const generation = this._formGeneration;
    this.setState({ testing: true, testResult: null });
    let testResult: ProviderState['testResult'];
    try {
      const report = await testProvider(this.state.draft, this.state.keyInput);
      // The engine's `local` covers private addresses too; "this computer" is loopback only.
      testResult = {
        ok: true,
        message: AI.isLoopback(this.state.draft)
          ? localized(
              '%1$@ answered on this computer in %2$@ ms.',
              report.model,
              `${report.latencyMs}`
            )
          : localized(
              '%1$@ answered from %2$@ in %3$@ ms.',
              report.model,
              report.host,
              `${report.latencyMs}`
            ),
      };
    } catch (err) {
      testResult = { ok: false, message: errorMessage(err, AI.providerName(this.state.draft)) };
    }
    if (!this._mounted) return;
    if (generation === this._formGeneration) {
      this.setState({ testing: false, testResult });
    } else {
      this.setState({ testing: false });
    }
  };

  _status(): [StatusKind, string] | null {
    const { draft, hasKey, keyKnown, testResult, modelsError } = this.state;
    if ((testResult && !testResult.ok) || modelsError) return ['bad', localized('Error')];
    if (AI.keyRequired(draft) && !hasKey) {
      return keyKnown ? ['warn', localized('Needs a key')] : null;
    }
    if (!(draft.model || '').trim()) return ['warn', localized('Needs a model')];
    return ['ok', localized('Ready')];
  }

  _renderText(field: 'name' | 'url', label: string, placeholder: string) {
    return (
      <SettingsField label={label} htmlFor={this._id(field)}>
        <input
          id={this._id(field)}
          type={field === 'url' ? 'url' : 'text'}
          spellCheck={false}
          value={this.state.draft[field] || ''}
          placeholder={placeholder}
          onChange={this._onChangeField(field)}
          onBlur={this._onCommit}
          onKeyDown={(e) => e.key === 'Enter' && this._onCommit()}
        />
      </SettingsField>
    );
  }

  _renderKey() {
    const { hasKey, keyInput, keyNotice, draft } = this.state;
    let placeholder = AI.keyRequired(draft) ? localized('Required') : localized('Optional');
    if (hasKey) placeholder = localized('Saved. Type a new key to replace it.');
    return (
      <SettingsField label={localized('API key')} htmlFor={this._id('key')}>
        <div className="ai-inline">
          <input
            id={this._id('key')}
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={keyInput}
            placeholder={placeholder}
            onChange={this._onChangeKey}
            onKeyDown={(e) => e.key === 'Enter' && this._onSaveKey()}
          />
          {!!keyInput.trim() && (
            <button className="btn btn-emphasis" onClick={this._onSaveKey}>
              {localized('Save key')}
            </button>
          )}
          {hasKey && !keyInput && (
            <button className="btn" onClick={this._onForgetKey}>
              {localized('Forget key')}
            </button>
          )}
        </div>
        {keyNotice && <div className="ai-field-note">{keyNotice}</div>}
      </SettingsField>
    );
  }

  _renderModel() {
    const { draft, models, loadingModels } = this.state;
    const modelsList = this._id('models');
    return (
      <>
        <SettingsField label={localized('Model')} htmlFor={this._id('model')}>
          <div className="ai-inline">
            <input
              id={this._id('model')}
              type="text"
              spellCheck={false}
              value={draft.model || ''}
              placeholder={localized('Required')}
              list={modelsList}
              onChange={this._onChangeField('model')}
              onBlur={this._onCommit}
              onKeyDown={(e) => e.key === 'Enter' && this._onCommit()}
            />
            <button className="btn" onClick={this._onLoadModels} disabled={loadingModels}>
              {loadingModels ? localized('Loading…') : localized('Load models')}
            </button>
          </div>
          <datalist id={modelsList}>
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </datalist>
        </SettingsField>
        {models.length > 0 && (
          <SettingsField label={localized('Available models')} htmlFor={this._id('model-picker')}>
            <select id={this._id('model-picker')} value={draft.model} onChange={this._onPickModel}>
              <option value="">{localized('Choose…')}</option>
              {models.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name && m.name !== m.id ? `${m.name} (${m.id})` : m.id}
                </option>
              ))}
            </select>
          </SettingsField>
        )}
      </>
    );
  }

  render() {
    const { draft, open, modelsError, testing, testResult } = this.state;
    const status = this._status();
    const model = (this.props.provider.model || '').trim();
    return (
      <ProviderCard
        logo={
          <ProviderLogo
            text={AI.providerName(draft).charAt(0).toUpperCase()}
            color={LOGO_COLORS[draft.kind]}
          />
        }
        title={AI.providerName(draft)}
        subtitle={model ? `${placeOf(draft)} · ${model}` : placeOf(draft)}
        status={status && <StatusPill kind={status[0]}>{status[1]}</StatusPill>}
        open={open}
        onToggle={(next) => this.setState({ open: next })}
      >
        {this._renderText('name', localized('Name'), AI.KINDS[draft.kind].name)}
        {this._renderText('url', localized('Address'), urlPlaceholder(draft.kind))}
        {draft.kind !== 'ollama' && this._renderKey()}
        {this._renderModel()}
        <SettingsField label={localized('Context window')} htmlFor={this._id('contextTokens')}>
          <div className="ai-inline">
            <input
              id={this._id('contextTokens')}
              className="ai-tokens"
              type="number"
              min={1024}
              step={1024}
              value={draft.contextTokens || ''}
              placeholder={draft.kind === 'ollama' ? '8192' : '16384'}
              onChange={this._onChangeField('contextTokens')}
              onBlur={this._onCommit}
            />
            <span className="ai-field-unit">{localized('tokens')}</span>
          </div>
        </SettingsField>
        <div className="settings-actions">
          <button className="btn" onClick={this._onTest} disabled={testing}>
            {testing ? localized('Testing…') : localized('Test')}
          </button>
          {modelsError && <span className="ai-result failed">{modelsError}</span>}
          {testResult && (
            <span className={`ai-result ${testResult.ok ? 'ok' : 'failed'}`}>
              {testResult.message}
            </span>
          )}
          <span className="spacer" />
          <button className="btn btn-danger" onClick={this.props.onRemove}>
            {localized('Remove')}
          </button>
        </div>
        <PrivacyFor provider={draft} />
      </ProviderCard>
    );
  }
}

interface TriageProgress {
  done: number;
  total: number;
  running: boolean;
  error?: string;
}

interface State {
  settings: AiSettings;
  /** "Sort now" by account id. */
  triage: { [accountId: string]: TriageProgress };
  /** `ai.usage` for the last 7 days; null when the engine doesn't keep it. */
  usage: AiUsageRow[] | null;
  showSent: boolean;
}

/** The local day as the engine keys usage: YYYY-MM-DD. */
function dayKey(date: Date) {
  const pad = (n: number) => `${n}`.padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** `core.ai.limits.dailyTokens`, set with `tern settings set`; the engine enforces it. */
function dailyCap(providerId: string): number | null {
  const caps = AppEnv.config.get('core.ai.limits.dailyTokens');
  const cap = caps && typeof caps === 'object' ? caps[providerId] : undefined;
  return typeof cap === 'number' && cap > 0 ? cap : null;
}

export default class PreferencesAI extends React.Component<Record<string, unknown>, State> {
  static displayName = 'PreferencesAI';

  _mounted = false;
  _unlisten: Array<() => void> = [];

  state: State = { settings: AI.readSettings(), triage: {}, usage: null, showSent: false };

  _runs = new Map<string, TriageRun>();

  componentDidMount() {
    this._mounted = true;
    const config = AppEnv.config.onDidChange(AI.CONFIG_KEY, () =>
      this.setState({ settings: AI.readSettings() })
    );
    Engine.events.on('connected', this._loadUsage);
    this._unlisten = [
      () => config.dispose(),
      () => Engine.events.removeListener('connected', this._loadUsage),
      CapabilityStore.listen(() => this.forceUpdate()),
      AccountStore.listen(() => this.forceUpdate()),
    ];
    this._loadUsage();
  }

  componentWillUnmount() {
    this._mounted = false;
    for (const unlisten of this._unlisten) unlisten();
    // A job runs on in the engine; only its progress stops showing.
    for (const run of this._runs.values()) run.dispose();
    this._runs.clear();
  }

  // The engine answers ai.usage only with its settings store, which comes with ai-triage.
  _loadUsage = async () => {
    if (!Engine.has(TRIAGE_ENGINE_FEATURE)) return;
    let rows: AiUsageRow[] | null = null;
    try {
      const reply = await Engine.request<AiUsageRow[]>('ai.usage', { days: 7 });
      if (Array.isArray(reply)) rows = reply;
    } catch (err) {
      // No usage to show.
    }
    if (this._mounted && rows) this.setState({ usage: rows });
  };

  _setTriage(accountId: string, next: (current: TriageProgress) => TriageProgress) {
    if (!this._runs.has(accountId)) return;
    this.setState(({ triage }) => ({
      triage: {
        ...triage,
        [accountId]: next(triage[accountId] || { done: 0, total: 0, running: true }),
      },
    }));
  }

  _onTriageInbox = (accountId: string) => {
    const run = triageInbox(accountId, (done, total) =>
      this._setTriage(accountId, () => ({ done, total, running: done < total }))
    );
    this._runs.set(accountId, run);
    this._setTriage(accountId, () => ({ done: 0, total: 0, running: true }));
    run.started.then(
      ({ queued }) =>
        this._setTriage(accountId, (p) => ({
          ...p,
          total: queued,
          running: p.running && queued > 0,
        })),
      () => {}
    );
    const end = (error?: string) => {
      this._setTriage(accountId, (p) => ({
        ...p,
        done: error === undefined ? p.total : p.done,
        running: false,
        error,
      }));
      this._runs.delete(accountId);
    };
    run.finished.then(
      () => end(),
      (err) => end(isCancelled(err) ? '' : errorMessage(err))
    );
  };

  _onStopTriage = (accountId: string) => {
    const run = this._runs.get(accountId);
    if (run) run.cancel();
  };

  _save(settings: AiSettings) {
    AI.writeSettings(settings);
    this.setState({ settings: AI.readSettings() });
  }

  _onAddProvider = (event: React.ChangeEvent<HTMLSelectElement>) => {
    const kind = event.target.value as AiProviderKind;
    if (!kind) return;
    const { settings } = this.state;
    const provider: AiProvider = {
      id: AI.newProviderId(kind, settings.providers),
      kind,
      model: '',
    };
    const features = { ...settings.features };
    // The first provider serves every feature that has none yet.
    for (const f of AI.AI_FEATURES) {
      if (!features[f].provider) features[f] = { ...features[f], provider: provider.id };
    }
    this._save({ ...settings, providers: [...settings.providers, provider], features });
  };

  _onChangeProvider = (provider: AiProvider) => {
    const { settings } = this.state;
    this._save({
      ...settings,
      providers: settings.providers.map((p) => (p.id === provider.id ? provider : p)),
    });
  };

  /** Drops the provider, its key, and its place in the features that used it. */
  _onRemoveProvider = async (provider: AiProvider) => {
    try {
      await forgetKey(provider.id);
    } catch (err) {
      // The engine is unreachable: keep the provider, so its key can still be forgotten.
      AppEnv.showErrorDialog(
        localized('Could not remove the saved API key: %@', (err && err.message) || err)
      );
      return;
    }
    const settings = AI.readSettings();
    const features = { ...settings.features };
    for (const f of AI.AI_FEATURES) {
      if (features[f].provider === provider.id) features[f] = { enabled: features[f].enabled };
    }
    this._save({
      ...settings,
      providers: settings.providers.filter((p) => p.id !== provider.id),
      features,
    });
  };

  _onToggleFeature = (feature: AiFeature) => {
    const { settings } = this.state;
    const current = settings.features[feature];
    this._save({
      ...settings,
      features: { ...settings.features, [feature]: { ...current, enabled: !current.enabled } },
    });
  };

  _onFeatureProvider = (feature: AiFeature, provider: string) => {
    const { settings } = this.state;
    const current = settings.features[feature];
    const next = provider ? { ...current, provider } : { enabled: current.enabled };
    this._save({ ...settings, features: { ...settings.features, [feature]: next } });
  };

  _onToggleAccount = (accountId: string) => {
    const { settings } = this.state;
    const accounts = settings.accounts.includes(accountId)
      ? settings.accounts.filter((a) => a !== accountId)
      : [...settings.accounts, accountId];
    this._save({ ...settings, accounts });
  };

  _onToggleAddresses = () => {
    const { settings } = this.state;
    this._save({ ...settings, includeAddresses: !settings.includeAddresses });
  };

  _renderProviders() {
    const { providers } = this.state.settings;
    return (
      <SettingsSection
        title={localized('Models')}
        note={
          <div className="ai-add-model">
            <select
              id="ai-add-provider"
              className="settings-select"
              aria-label={localized('Add a model')}
              value=""
              onChange={this._onAddProvider}
            >
              <option value="">{localized('+ Add a model')}</option>
              {KIND_ORDER.map((kind) => (
                <option key={kind} value={kind}>
                  {AI.KINDS[kind].name}
                </option>
              ))}
            </select>
            <span>
              {localized(
                'Anthropic, OpenAI, any OpenAI-compatible server like LM Studio, or Ollama'
              )}
            </span>
          </div>
        }
      >
        {providers.length === 0 && (
          <div className="ai-empty">
            {localized(
              'No model yet. Add a cloud model with your own API key, or one that runs on this computer with Ollama, LM Studio or llama.cpp.'
            )}
          </div>
        )}
        {providers.map((p) => (
          <ProviderSection
            key={p.id}
            provider={p}
            onChange={this._onChangeProvider}
            onRemove={() => this._onRemoveProvider(p)}
          />
        ))}
      </SettingsSection>
    );
  }

  _renderFeatures() {
    const { settings } = this.state;
    return (
      <SettingsSection title={localized('Features')}>
        {FEATURES.map(({ feature, label, description, capability }) => {
          const f = settings.features[feature];
          const status = CapabilityStore.get(capability);
          return (
            <SettingsRow
              key={feature}
              label={label()}
              description={
                <>
                  {description()}
                  {f.enabled && !status.available && status.reason && (
                    <span className="ai-status">{status.reason}.</span>
                  )}
                </>
              }
            >
              <select
                id={`ai-feature-${feature}-provider`}
                className="settings-select"
                aria-label={localized('Model for %@', label())}
                value={f.provider || ''}
                onChange={(e) => this._onFeatureProvider(feature, e.target.value)}
              >
                <option value="">{localized('No model')}</option>
                {settings.providers.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.model ? `${AI.providerName(p)}: ${p.model}` : AI.providerName(p)}
                  </option>
                ))}
              </select>
              <SettingsSwitch
                checked={f.enabled}
                label={label()}
                onChange={() => this._onToggleFeature(feature)}
              />
            </SettingsRow>
          );
        })}
        {this._renderTriageRows()}
      </SettingsSection>
    );
  }

  _renderTriageRows() {
    const { settings, triage } = this.state;
    if (!settings.features.triage.enabled || !Engine.has(TRIAGE_ENGINE_FEATURE)) return null;
    const accounts = AccountStore.accounts().filter((a) => settings.accounts.includes(a.id));
    return accounts.map((account) => {
      const progress = triage[account.id];
      let progressText = '';
      if (progress) {
        progressText =
          progress.error ||
          (progress.running
            ? localized('%1$@ of %2$@', `${progress.done}`, `${progress.total}`)
            : localized('%@ threads sorted', `${progress.done}`));
      }
      return (
        <SettingsRow
          key={`triage-${account.id}`}
          className="ai-triage-account"
          label={
            accounts.length > 1
              ? localized('Sort the last 7 days in %@', account.emailAddress)
              : localized('Sort the last 7 days')
          }
          description={localized(
            'One pass over inbox mail that arrived before you turned sorting on. Your own changes are kept.'
          )}
        >
          {progressText && <span className="ai-triage-progress">{progressText}</span>}
          {progress && progress.running ? (
            <button className="btn" onClick={() => this._onStopTriage(account.id)}>
              {localized('Stop')}
            </button>
          ) : (
            <button className="btn" onClick={() => this._onTriageInbox(account.id)}>
              {localized('Sort now')}
            </button>
          )}
        </SettingsRow>
      );
    });
  }

  _renderAccounts() {
    const { settings } = this.state;
    return (
      <SettingsSection
        title={localized('Which mail')}
        note={localized(
          'Mail is only sent to a model from the accounts you turn on here, and only for the features you turn on.'
        )}
      >
        {AccountStore.accounts().map((account) => {
          const named = account.label && account.label !== account.emailAddress;
          const name = named ? account.label : account.emailAddress;
          return (
            <SettingsRow
              key={account.id}
              label={name}
              description={named ? account.emailAddress : undefined}
            >
              <SettingsSwitch
                checked={settings.accounts.includes(account.id)}
                label={name}
                onChange={() => this._onToggleAccount(account.id)}
              />
            </SettingsRow>
          );
        })}
        <SettingsRow
          label={localized('Include email addresses')}
          description={localized(
            'Off: the model sees names instead of email addresses, in the people list and in the text.'
          )}
        >
          <SettingsSwitch
            checked={settings.includeAddresses}
            label={localized('Include email addresses')}
            onChange={this._onToggleAddresses}
          />
        </SettingsRow>
      </SettingsSection>
    );
  }

  _renderUsage() {
    const { usage, settings } = this.state;
    if (!usage) return null;
    const today = dayKey(new Date());
    const days: string[] = [];
    for (let i = 6; i >= 0; i--) {
      const d = new Date();
      d.setDate(d.getDate() - i);
      days.push(dayKey(d));
    }
    const tokensByDay = new Map<string, number>();
    const byProvider = new Map<string, { tokens: number; requests: number }>();
    let requests = 0;
    let tokens = 0;
    for (const r of usage) {
      const t = r.inputTokens + r.outputTokens;
      tokensByDay.set(r.day, (tokensByDay.get(r.day) || 0) + t);
      if (r.day !== today) continue;
      requests += r.requests;
      tokens += t;
      const p = byProvider.get(r.provider) || { tokens: 0, requests: 0 };
      p.tokens += t;
      p.requests += r.requests;
      byProvider.set(r.provider, p);
    }
    const peak = Math.max(1, ...days.map((d) => tokensByDay.get(d) || 0));
    const nameOf = (id: string) => {
      const provider = settings.providers.find((p) => p.id === id);
      return provider ? AI.providerName(provider) : id;
    };
    // Providers with a cap show it even before their first request of the day.
    for (const p of settings.providers) {
      if (dailyCap(p.id) && !byProvider.has(p.id)) byProvider.set(p.id, { tokens: 0, requests: 0 });
    }
    const perProvider = [...byProvider].map(([id, p]) => {
      const cap = dailyCap(id);
      return cap
        ? localized(
            '%1$@: %2$@ of %3$@ tokens a day.',
            nameOf(id),
            p.tokens.toLocaleString(),
            cap.toLocaleString()
          )
        : localized('%1$@: %2$@ tokens.', nameOf(id), p.tokens.toLocaleString());
    });
    return (
      <SettingsSection title={localized('Usage today')}>
        <SettingsRow
          label={localized(
            '%1$@ requests · %2$@ tokens',
            requests.toLocaleString(),
            tokens.toLocaleString()
          )}
          description={perProvider.length ? perProvider.join(' ') : undefined}
        >
          <div className="ai-usage-bars" aria-label={localized('Tokens a day, last 7 days')}>
            {days.map((d) => (
              <i
                key={d}
                title={`${d}: ${(tokensByDay.get(d) || 0).toLocaleString()}`}
                style={{ height: `${Math.max(6, ((tokensByDay.get(d) || 0) / peak) * 100)}%` }}
              />
            ))}
          </div>
        </SettingsRow>
      </SettingsSection>
    );
  }

  _renderSent() {
    const { showSent } = this.state;
    return (
      <SettingsSection
        bare
        className="ai-sent"
        title={
          <button
            className="ai-disclosure"
            aria-expanded={showSent}
            onClick={() => this.setState({ showSent: !showSent })}
          >
            <span className="settings-chevron" aria-hidden />
            {localized('What is sent to the model')}
          </button>
        }
      >
        {showSent && (
          <>
            <SettingsCard>
              <table className="ai-sent-table">
                <thead>
                  <tr>
                    <th>{localized('Feature')}</th>
                    <th>{localized('What leaves Tern')}</th>
                    <th>{localized('When')}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <td>{localized('Summary')}</td>
                    <td>
                      {localized(
                        "The thread's subject, sender names, dates and the plain text of its messages, without quoted replies and signatures"
                      )}
                    </td>
                    <td>{localized('When you click Summarize')}</td>
                  </tr>
                  <tr>
                    <td>{localized('Reply suggestions')}</td>
                    <td>{localized('The same, plus what you ask for and your name')}</td>
                    <td>
                      {localized('When you click Get Suggestions or Write, or pick a suggestion')}
                    </td>
                  </tr>
                  <tr>
                    <td>{localized('Inbox sorting')}</td>
                    <td>
                      {localized(
                        "The subject, sender names and the plain text of the thread's newest message you received"
                      )}
                    </td>
                    <td>
                      {localized('When new mail arrives in the inbox, or you click Sort now')}
                    </td>
                  </tr>
                </tbody>
              </table>
            </SettingsCard>
            <SettingsNote>
              {localized(
                'Mail goes from this computer to the model you chose, and nowhere else. Attachments are never sent, only their file names. Spam, Trash, drafts and encrypted messages are always left out. Summaries are kept in memory only, and sorting results on this computer only.'
              )}
            </SettingsNote>
          </>
        )}
      </SettingsSection>
    );
  }

  render() {
    return (
      <SettingsPage
        className="preferences-ai"
        title={localized('AI')}
        description={localized(
          'Summaries, reply suggestions and inbox sorting with a model you choose. Nothing is sent until you turn on a feature and an account.'
        )}
      >
        {this._renderProviders()}
        {this._renderFeatures()}
        {this._renderAccounts()}
        {this._renderUsage()}
        {this._renderSent()}
      </SettingsPage>
    );
  }
}
