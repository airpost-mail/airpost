// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import React from 'react';
import {
  AI,
  AccountStore,
  CapabilityStore,
  KeyManager,
  localized,
  AiFeature,
  AiModelInfo,
  AiProvider,
  AiProviderKind,
  AiSettings,
  Engine,
} from 'mailspring-exports';
import { destinationNote, errorMessage, isCancelled, listModels, testProvider } from './ai-client';
import { TRIAGE_ENGINE_FEATURE, TriageRun, triageInbox } from './triage';

// Preferences > AI (docs/ai-plan.md section 7). Settings go to config under core.ai; API keys only
// to the keyset as integration-ai-<id>-key, and are never shown again once saved.

const KIND_ORDER: AiProviderKind[] = ['anthropic', 'openai', 'ollama'];

const FEATURES: {
  feature: AiFeature;
  label: () => string;
  capability: 'ai-summary' | 'ai-reply' | 'ai-triage';
}[] = [
  { feature: 'summary', label: () => localized('Thread summaries'), capability: 'ai-summary' },
  { feature: 'reply', label: () => localized('Reply suggestions'), capability: 'ai-reply' },
  { feature: 'triage', label: () => localized('Inbox triage'), capability: 'ai-triage' },
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

interface ProviderProps {
  provider: AiProvider;
  onChange: (provider: AiProvider) => void;
  onRemove: () => Promise<void> | void;
}

interface ProviderState {
  draft: AiProvider;
  keyInput: string;
  hasKey: boolean;
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
      keyInput: '',
      hasKey: false,
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
  }

  componentDidUpdate(prev: ProviderProps) {
    if (JSON.stringify(prev.provider) !== JSON.stringify(this.props.provider)) {
      this.setState({ draft: { ...this.props.provider } });
    }
  }

  componentWillUnmount() {
    this._mounted = false;
  }

  _id(field: string) {
    return `ai-provider-${this.props.provider.id}-${field}`;
  }

  async _loadKeyPresence() {
    const key = await KeyManager.getPassword(AI.keyName(this.props.provider.id));
    if (this._mounted) this.setState({ hasKey: !!(key || '').trim() });
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
      const name = AI.keyName(previous.id);
      let saved = true;
      try {
        saved = !!(await KeyManager.getPassword(name));
      } catch (err) {
        // Unreadable: remove it anyway.
      }
      if (saved) {
        await KeyManager.deletePassword(name);
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

  /** Saves the typed key on Save Key or Enter only, never on blur: Test must not store a key
   * before it was tested. */
  _onSaveKey = async () => {
    const value = this.state.keyInput.trim();
    if (!value) return;
    await KeyManager.replacePassword(AI.keyName(this.props.provider.id), value);
    if (this._mounted && this.state.keyInput.trim() === value) {
      this.setState({ keyInput: '', hasKey: true, keyNotice: '' });
    }
  };

  _onForgetKey = async () => {
    this._formGeneration++;
    await KeyManager.deletePassword(AI.keyName(this.props.provider.id));
    if (this._mounted) this.setState({ keyInput: '', hasKey: false, testResult: null });
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

  _renderText(field: 'name' | 'url' | 'model', label: string, placeholder: string, list?: string) {
    const value = this.state.draft[field];
    return (
      <div className="item ai-field">
        <label htmlFor={this._id(field)}>{label}:</label>
        <input
          id={this._id(field)}
          type={field === 'url' ? 'url' : 'text'}
          spellCheck={false}
          value={value || ''}
          placeholder={placeholder}
          list={list}
          onChange={this._onChangeField(field)}
          onBlur={this._onCommit}
          onKeyDown={(e) => e.key === 'Enter' && this._onCommit()}
        />
      </div>
    );
  }

  _renderKey() {
    const { hasKey, keyInput, keyNotice, draft } = this.state;
    let placeholder = AI.keyRequired(draft) ? localized('Required') : localized('Optional');
    if (hasKey) placeholder = localized('Saved. Type a new key to replace it.');
    return (
      <div className="item ai-field">
        <label htmlFor={this._id('key')}>{localized('API key')}:</label>
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
          <button className="btn btn-small" onClick={this._onSaveKey}>
            {localized('Save Key')}
          </button>
        )}
        {hasKey && !keyInput && (
          <button className="btn btn-small" onClick={this._onForgetKey}>
            {localized('Forget Key')}
          </button>
        )}
        {keyNotice && <span className="ai-key-notice">{keyNotice}</span>}
      </div>
    );
  }

  render() {
    const { draft, models, loadingModels, modelsError, testing, testResult } = this.state;
    const modelsList = this._id('models');
    return (
      <section className={`ai-provider ai-provider-${draft.kind}`}>
        <h6>
          {AI.providerName(draft)}
          <span className="ai-provider-kind">{AI.KINDS[draft.kind].name}</span>
        </h6>
        {this._renderText('name', localized('Name'), AI.KINDS[draft.kind].name)}
        {this._renderText('url', localized('Server URL'), urlPlaceholder(draft.kind))}
        {draft.kind !== 'ollama' && this._renderKey()}
        {this._renderText('model', localized('Model'), localized('Required'), modelsList)}
        <datalist id={modelsList}>
          {models.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
        </datalist>
        {models.length > 0 && (
          <div className="item ai-field">
            <label htmlFor={this._id('model-picker')}>{localized('Available models')}:</label>
            <select id={this._id('model-picker')} value={draft.model} onChange={this._onPickModel}>
              <option value="">{localized('Choose…')}</option>
              {models.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name && m.name !== m.id ? `${m.name} (${m.id})` : m.id}
                </option>
              ))}
            </select>
          </div>
        )}
        <div className="item ai-field">
          <label htmlFor={this._id('contextTokens')}>{localized('Context window')}:</label>
          <input
            id={this._id('contextTokens')}
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
        <div className="item ai-actions">
          <button className="btn" onClick={this._onLoadModels} disabled={loadingModels}>
            {loadingModels ? localized('Loading…') : localized('Load Models')}
          </button>
          <button className="btn" onClick={this._onTest} disabled={testing}>
            {testing ? localized('Testing…') : localized('Test')}
          </button>
          <button className="btn" onClick={this.props.onRemove}>
            {localized('Remove')}
          </button>
          {modelsError && <span className="ai-test-result failed">{modelsError}</span>}
          {testResult && (
            <span className={`ai-test-result ${testResult.ok ? 'ok' : 'failed'}`}>
              {testResult.message}
            </span>
          )}
        </div>
        <p className="ai-privacy">{destinationNote(draft)}</p>
      </section>
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
  /** "Triage inbox" by account id. */
  triage: { [accountId: string]: TriageProgress };
}

export default class PreferencesAI extends React.Component<Record<string, unknown>, State> {
  static displayName = 'PreferencesAI';

  _unlisten: Array<() => void> = [];

  state: State = { settings: AI.readSettings(), triage: {} };

  _runs = new Map<string, TriageRun>();

  componentDidMount() {
    const config = AppEnv.config.onDidChange(AI.CONFIG_KEY, () =>
      this.setState({ settings: AI.readSettings() })
    );
    this._unlisten = [
      () => config.dispose(),
      CapabilityStore.listen(() => this.forceUpdate()),
      AccountStore.listen(() => this.forceUpdate()),
    ];
  }

  componentWillUnmount() {
    for (const unlisten of this._unlisten) unlisten();
    // A job runs on in the engine; only its progress stops showing.
    for (const run of this._runs.values()) run.dispose();
    this._runs.clear();
  }

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
    await KeyManager.deletePassword(AI.keyName(provider.id));
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
      <section className="ai-section ai-providers">
        <h6>{localized('Models')}</h6>
        {providers.length === 0 && (
          <p>
            {localized(
              'Add a provider: a cloud API with your own key, or a model running on this computer with Ollama, LM Studio or llama.cpp.'
            )}
          </p>
        )}
        {providers.map((p) => (
          <ProviderSection
            key={p.id}
            provider={p}
            onChange={this._onChangeProvider}
            onRemove={() => this._onRemoveProvider(p)}
          />
        ))}
        <div className="item ai-field">
          <label htmlFor="ai-add-provider">{localized('Add provider')}:</label>
          <select id="ai-add-provider" value="" onChange={this._onAddProvider}>
            <option value="">{localized('Choose…')}</option>
            {KIND_ORDER.map((kind) => (
              <option key={kind} value={kind}>
                {AI.KINDS[kind].name}
              </option>
            ))}
          </select>
        </div>
      </section>
    );
  }

  _renderFeatures() {
    const { settings } = this.state;
    return (
      <section className="ai-section ai-features">
        <h6>{localized('Features')}</h6>
        {FEATURES.map(({ feature, label, capability }) => {
          const f = settings.features[feature];
          const status = CapabilityStore.get(capability);
          return (
            <div key={feature} className="ai-feature">
              <div className="item ai-field">
                <input
                  id={`ai-feature-${feature}`}
                  type="checkbox"
                  checked={f.enabled}
                  onChange={() => this._onToggleFeature(feature)}
                />
                <label htmlFor={`ai-feature-${feature}`}>{label()}</label>
                <select
                  id={`ai-feature-${feature}-provider`}
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
              </div>
              {f.enabled && !status.available && status.reason && (
                <p className="ai-status">{status.reason}.</p>
              )}
            </div>
          );
        })}
      </section>
    );
  }

  _renderTriage() {
    const { settings, triage } = this.state;
    if (!settings.features.triage.enabled || !Engine.has(TRIAGE_ENGINE_FEATURE)) return null;
    const accounts = AccountStore.accounts().filter((a) => settings.accounts.includes(a.id));
    return (
      <section className="ai-section ai-triage">
        <h6>{localized('Triage')}</h6>
        <p>
          {localized(
            'New mail in the inbox of the accounts below is sorted as it arrives. Threads that need a reply or look important get a chip in the list; click it to change the category.'
          )}
        </p>
        {accounts.map((account) => {
          const progress = triage[account.id];
          return (
            <div key={account.id} className="item ai-field ai-triage-account">
              <label>{account.emailAddress}</label>
              {progress && progress.running ? (
                <button className="btn" onClick={() => this._onStopTriage(account.id)}>
                  {localized('Stop')}
                </button>
              ) : (
                <button className="btn" onClick={() => this._onTriageInbox(account.id)}>
                  {localized('Triage inbox')}
                </button>
              )}
              {progress && (
                <span className="ai-triage-progress">
                  {progress.error ||
                    (progress.running
                      ? localized('%1$@ of %2$@', `${progress.done}`, `${progress.total}`)
                      : localized('%@ threads triaged', `${progress.done}`))}
                </span>
              )}
            </div>
          );
        })}
        <p className="ai-hint">
          {localized(
            'Triage inbox sorts the inbox threads of the last 7 days. Your own changes are kept.'
          )}
        </p>
      </section>
    );
  }

  _renderAccounts() {
    const { settings } = this.state;
    const accounts = AccountStore.accounts();
    return (
      <section className="ai-section ai-accounts">
        <h6>{localized('Accounts')}</h6>
        <p>{localized('Mail is only sent to a model from the accounts you turn on here.')}</p>
        {accounts.map((account) => (
          <div key={account.id} className="item">
            <input
              id={`ai-account-${account.id}`}
              type="checkbox"
              checked={settings.accounts.includes(account.id)}
              onChange={() => this._onToggleAccount(account.id)}
            />
            <label htmlFor={`ai-account-${account.id}`}>
              {account.label && account.label !== account.emailAddress
                ? `${account.label} (${account.emailAddress})`
                : account.emailAddress}
            </label>
          </div>
        ))}
        <div className="item">
          <input
            id="ai-include-addresses"
            type="checkbox"
            checked={settings.includeAddresses}
            onChange={this._onToggleAddresses}
          />
          <label htmlFor="ai-include-addresses">{localized('Include email addresses')}</label>
        </div>
        <p className="ai-hint">
          {localized(
            'Off: the model sees names instead of email addresses, in the people list and in the text.'
          )}
        </p>
      </section>
    );
  }

  _renderPrivacy() {
    return (
      <section className="ai-section ai-privacy-table">
        <h6>{localized('What is sent')}</h6>
        <table>
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
              <td>{localized('When you click Get Suggestions or Write, or pick a suggestion')}</td>
            </tr>
            <tr>
              <td>{localized('Inbox triage')}</td>
              <td>
                {localized(
                  "The subject, sender names and the plain text of the thread's newest message you received"
                )}
              </td>
              <td>{localized('When new mail arrives in the inbox, or you click Triage inbox')}</td>
            </tr>
          </tbody>
        </table>
        <p className="ai-hint">
          {localized(
            'Mail goes from this computer to the model you chose, and nowhere else. Attachments are never sent, only their file names. Spam, Trash, drafts and encrypted messages are always left out. Summaries are kept in memory only, and triage results on this computer only.'
          )}
        </p>
      </section>
    );
  }

  render() {
    return (
      <div className="container-ai">
        <p className="ai-intro">
          {localized(
            'Tern can summarize threads, suggest replies and sort new mail with a model you choose. Nothing is sent until you turn on a feature and an account. API keys are stored encrypted, like your account passwords.'
          )}
        </p>
        {this._renderProviders()}
        {this._renderFeatures()}
        {this._renderAccounts()}
        {this._renderTriage()}
        {this._renderPrivacy()}
      </div>
    );
  }
}
