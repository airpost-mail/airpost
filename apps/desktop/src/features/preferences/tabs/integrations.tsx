import React from 'react';
import * as Actions from '../../../data/actions';
import { localized, localizedReactFragment } from '../../../intl/localize';
import KeyManager from '../../../window/key-manager';
import { CapabilityStore, CapabilityFeature } from '../../../data/stores/capability';
import {
  Integrations,
  IntegrationKind,
  IntegrationProvider,
  IntegrationSettings,
  ProviderConfig,
} from '../../../engine/client/integrations';
import {
  SettingsPage,
  SettingsSection,
  SettingsNote,
  SettingsField,
  StatusPill,
  StatusKind,
  ProviderCard,
  ProviderLogo,
  PrivacyNote,
} from '../../../components/settings';
import { Button } from '../../../components/kit/button';
import { TextField } from '../../../components/kit/text-field';
import { SettingsSelect } from '../../../components/settings-select';

// Preferences > Integrations. Settings go to config under
// core.integrations.<kind>; API keys to the engine's secret store (the keyset for an engine
// without account setup), and are never shown again once saved.

const FEATURE_FOR_KIND: { [kind in IntegrationKind]: CapabilityFeature } = {
  translation: 'translation',
  grammar: 'grammar-check',
  enrichment: 'contact-enrichment',
};

function hostOf(url: string | undefined, fallback: string) {
  try {
    return url ? new URL(url).host : fallback;
  } catch (err) {
    return fallback;
  }
}

const LANGUAGETOOL_PREMIUM_URL = 'https://api.languagetoolplus.com';

/** The LanguageTool server drafts go to, by the rule of LanguageToolConfig::base_url in
 * crates/tern-integrations: premium credentials with the default URL go to the premium host. */
function languageToolUrl(settings: IntegrationSettings, hasKey: boolean) {
  let url = (settings.url || '').trim().replace(/\/+$/, '');
  if (url.endsWith('/v2')) url = url.slice(0, -'/v2'.length);
  if (!url || url === Integrations.LANGUAGETOOL_DEFAULT_URL) {
    const premium = !!(settings.username || '').trim() && hasKey;
    return premium ? LANGUAGETOOL_PREMIUM_URL : Integrations.LANGUAGETOOL_DEFAULT_URL;
  }
  return url;
}

export function privacyNote(kind: IntegrationKind, settings: IntegrationSettings, hasKey = false) {
  if (kind === 'translation') {
    const host =
      settings.provider === 'deepl'
        ? 'DeepL'
        : hostOf(settings.url, localized('your LibreTranslate server'));
    return localized('Message text you translate is sent to %@.', host);
  }
  if (kind === 'grammar') {
    return localized(
      'Drafts are sent to %@ while you type.',
      hostOf(languageToolUrl(settings, hasKey), 'LanguageTool')
    );
  }
  return localized('Email addresses of people you view are sent to People Data Labs.');
}

const LOOKS: {
  [kind in IntegrationKind]: {
    title: () => string;
    logo: (provider: IntegrationProvider | null) => { text: string; color: string };
  };
} = {
  translation: {
    title: () => localized('Translation'),
    logo: (provider) =>
      provider === 'libretranslate'
        ? { text: 'L', color: '#1f6f8b' }
        : { text: 'D', color: '#1d4f80' },
  },
  grammar: {
    title: () => localized('Grammar'),
    logo: () => ({ text: 'LT', color: '#3b7dd8' }),
  },
  enrichment: {
    title: () => localized('Contact details'),
    logo: () => ({ text: 'P', color: '#7b2ff7' }),
  },
};

/** The provider, and the server it talks to when that can differ. */
function subtitle(kind: IntegrationKind, settings: IntegrationSettings, hasKey: boolean) {
  const { provider } = settings;
  if (kind === 'enrichment') {
    return localized('People Data Labs: job titles, companies and profiles');
  }
  if (!provider) return localized('DeepL or LibreTranslate');
  const name = Integrations.PROVIDERS[provider].name;
  if (provider === 'languagetool') {
    return `${name} · ${hostOf(languageToolUrl(settings, hasKey), name)}`;
  }
  if (provider === 'libretranslate' && settings.url) {
    return `${name} · ${hostOf(settings.url, name)}`;
  }
  return name;
}

interface SectionProps {
  kind: IntegrationKind;
}

interface SectionState {
  settings: IntegrationSettings;
  /** A key typed but not saved yet. */
  keyInput: string;
  hasKey: boolean;
  /** No provider chosen, but the keyset still has a key for one of this kind's providers. */
  hasLeftoverKey: boolean;
  testing: boolean;
  testResult: { ok: boolean; message: string } | null;
  /** Fields stay hidden until Set up, or while the card is closed. */
  open: boolean;
}

export class IntegrationSection extends React.Component<SectionProps, SectionState> {
  static displayName = 'IntegrationSection';

  _mounted = false;
  /** Bumped whenever the form changes, so a Test reply for an older form is dropped. */
  _formGeneration = 0;
  _unlisten?: () => void;

  constructor(props: SectionProps) {
    super(props);
    const settings = Integrations.readSettings(props.kind);
    this.state = {
      settings,
      keyInput: '',
      hasKey: false,
      hasLeftoverKey: false,
      testing: false,
      testResult: null,
      open: settings.enabled,
    };
  }

  componentDidMount() {
    this._mounted = true;
    this._unlisten = CapabilityStore.listen(() => this.forceUpdate());
    this._loadKeyPresence();
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._unlisten) this._unlisten();
  }

  async _loadKeyPresence(provider = this.state.settings.provider) {
    if (!provider) {
      this.setState({ hasKey: false });
      const keys = await Promise.all(
        Integrations.PROVIDERS_FOR_KIND[this.props.kind].map((p) => Integrations.hasKey(p))
      );
      if (this._mounted && !this.state.settings.provider) {
        this.setState({ hasLeftoverKey: keys.some((k) => k) });
      }
      return;
    }
    const hasKey = await Integrations.hasKey(provider);
    if (this._mounted && provider === this.state.settings.provider) {
      this.setState({ hasKey, hasLeftoverKey: false });
    }
  }

  _update(changes: Partial<IntegrationSettings>) {
    const settings = { ...this.state.settings, ...changes };
    Integrations.writeSettings(this.props.kind, settings);
    this.setState({ settings: Integrations.readSettings(this.props.kind), testResult: null });
  }

  _onChangeProvider = (value: string) => {
    const provider = (value || null) as IntegrationProvider | null;
    // URLs don't carry over between providers; keys are per provider in the keyset.
    this._formGeneration++;
    this._update({ provider, url: undefined, enabled: !!provider && this.state.settings.enabled });
    this.setState({ keyInput: '', hasKey: false });
    this._loadKeyPresence(provider);
  };

  _onToggleEnabled = () => {
    this._update({ enabled: !this.state.settings.enabled });
  };

  _onChangeField = (field: 'url' | 'username') => (event: React.ChangeEvent<HTMLInputElement>) => {
    this._formGeneration++;
    this.setState({ settings: { ...this.state.settings, [field]: event.target.value } });
  };

  _onCommitField = () => {
    this._update({});
  };

  _onChangeKey = (event: React.ChangeEvent<HTMLInputElement>) => {
    this._formGeneration++;
    this.setState({ keyInput: event.target.value, testResult: null });
  };

  /** Saves the typed key on Save Key or Enter only, never on blur: clicking Test must not store
   * a key before it has been tested. Saving keeps a pending Test's result, since the key is the
   * same. */
  _onSaveKey = async () => {
    const { provider } = this.state.settings;
    const value = this.state.keyInput.trim();
    if (!provider || !value) return;
    await Integrations.saveKey(provider, value);
    if (this._mounted && this.state.keyInput.trim() === value) {
      this.setState({ keyInput: '', hasKey: true });
    }
  };

  _onForgetKey = async () => {
    const { provider } = this.state.settings;
    if (!provider) return;
    this._formGeneration++;
    await Integrations.deleteKey(provider);
    if (this._mounted) this.setState({ keyInput: '', hasKey: false, testResult: null });
  };

  /** Clears the settings and deletes the saved keys of every provider of this kind, not just
   * the selected one. Turning it off keeps both. */
  _onRemove = async () => {
    this._formGeneration++;
    await Promise.all(
      Integrations.PROVIDERS_FOR_KIND[this.props.kind].map((p) => Integrations.deleteKey(p))
    );
    AppEnv.config.set(`core.integrations.${this.props.kind}`, {});
    if (this._mounted) {
      this.setState({
        settings: Integrations.readSettings(this.props.kind),
        keyInput: '',
        hasKey: false,
        hasLeftoverKey: false,
        testing: false,
        testResult: null,
      });
    }
  };

  /** Tests what the form shows, saved or not: the typed key wins over the saved one. */
  _onTest = async () => {
    const { settings } = this.state;
    if (!settings.provider) return;
    const generation = this._formGeneration;
    this.setState({ testing: true, testResult: null });
    const name = Integrations.PROVIDERS[settings.provider].name;
    let testResult: SectionState['testResult'];
    try {
      const config = await this._formConfig();
      const report = await Integrations.request('integrations.test', {}, { config });
      let message = localized('Connected to %@.', report.host);
      if (report.usage) {
        message +=
          ' ' +
          (report.usage.limit
            ? localized(
                '%1$@ of %2$@ characters used this period.',
                report.usage.used.toLocaleString(),
                report.usage.limit.toLocaleString()
              )
            : localized('%@ characters used this period.', report.usage.used.toLocaleString()));
      }
      testResult = { ok: true, message };
    } catch (err) {
      testResult = { ok: false, message: Integrations.errorMessage(err, name) };
    }
    if (this._mounted && generation === this._formGeneration) {
      this.setState({ testing: false, testResult });
    } else if (this._mounted) {
      this.setState({ testing: false });
    }
  };

  async _formConfig(): Promise<ProviderConfig | null> {
    const { settings, keyInput } = this.state;
    const typed = keyInput.trim();
    // Only read the keyset when the engine can take the request. Without a key the engine
    // uses the one it keeps.
    const key =
      typed ||
      (Integrations.isAvailable() && settings.provider
        ? await KeyManager.getPassword(Integrations.keyName(settings.provider))
        : '');
    return Integrations.buildProviderConfig(settings, key);
  }

  _renderKeyField(label: string, required: boolean) {
    const { hasKey, keyInput } = this.state;
    const id = `integrations-${this.props.kind}-key`;
    let placeholder = required ? localized('Required') : localized('Optional');
    if (hasKey) placeholder = localized('Saved. Type a new key to replace it.');
    return (
      <SettingsField label={label} htmlFor={id}>
        <div className="integrations-inline">
          <TextField
            id={id}
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={keyInput}
            placeholder={placeholder}
            onChange={this._onChangeKey}
            onKeyDown={(e) => e.key === 'Enter' && this._onSaveKey()}
          />
          {!!keyInput.trim() && (
            <Button variant="default" onClick={this._onSaveKey}>
              {localized('Save key')}
            </Button>
          )}
          {hasKey && !keyInput && (
            <Button variant="outline" onClick={this._onForgetKey}>
              {localized('Forget key')}
            </Button>
          )}
        </div>
      </SettingsField>
    );
  }

  _renderTextField(field: 'url' | 'username', label: string, placeholder: string) {
    const id = `integrations-${this.props.kind}-${field}`;
    return (
      <SettingsField label={label} htmlFor={id}>
        <TextField
          id={id}
          type={field === 'url' ? 'url' : 'text'}
          spellCheck={false}
          value={this.state.settings[field] || ''}
          placeholder={placeholder}
          onChange={this._onChangeField(field)}
          onBlur={this._onCommitField}
          onKeyDown={(e) => e.key === 'Enter' && this._onCommitField()}
        />
      </SettingsField>
    );
  }

  _renderFields() {
    const { provider } = this.state.settings;
    switch (provider) {
      case 'libretranslate':
        return (
          <>
            {this._renderTextField('url', localized('Server'), 'https://libretranslate.com')}
            {this._renderKeyField(localized('API key'), false)}
          </>
        );
      case 'deepl':
        return this._renderKeyField(localized('API key'), true);
      case 'languagetool':
        return (
          <>
            {this._renderTextField(
              'url',
              localized('Server'),
              Integrations.LANGUAGETOOL_DEFAULT_URL
            )}
            {this._renderTextField(
              'username',
              localized('Premium account'),
              localized('Optional, the email of your LanguageTool account')
            )}
            {this._renderKeyField(localized('Premium API key'), false)}
          </>
        );
      case 'peopledatalabs':
        return this._renderKeyField(localized('API key'), true);
      default:
        return null;
    }
  }

  _renderServiceField() {
    const { kind } = this.props;
    const choices = Integrations.PROVIDERS_FOR_KIND[kind];
    if (choices.length < 2) return null;
    const id = `integrations-${kind}-provider`;
    return (
      <SettingsField label={localized('Service')} htmlFor={id}>
        <SettingsSelect
          id={id}
          value={this.state.settings.provider || ''}
          onChange={this._onChangeProvider}
          options={[
            ['', localized('None')],
            ...choices.map((p): [string, string] => [p, Integrations.PROVIDERS[p].name]),
          ]}
        />
      </SettingsField>
    );
  }

  _renderBody() {
    const { kind } = this.props;
    const { settings, testing, testResult, hasKey, hasLeftoverKey, keyInput } = this.state;
    const capability = CapabilityStore.get(FEATURE_FOR_KIND[kind]);
    if (!settings.provider) {
      return (
        <>
          {this._renderServiceField()}
          {hasLeftoverKey && (
            <div className="settings-actions">
              <Button variant="outline" onClick={this._onRemove}>
                {localized('Remove saved keys')}
              </Button>
            </div>
          )}
        </>
      );
    }
    return (
      <>
        {this._renderServiceField()}
        {this._renderFields()}
        <div className="settings-actions">
          {settings.enabled ? (
            <Button variant="outline" onClick={this._onToggleEnabled}>
              {localized('Turn off')}
            </Button>
          ) : (
            <Button variant="default" onClick={this._onToggleEnabled}>
              {localized('Turn on')}
            </Button>
          )}
          <Button variant="outline" onClick={this._onTest} disabled={testing}>
            {testing ? localized('Testing…') : localized('Test')}
          </Button>
          {testResult && (
            <span className={`integrations-test-result ${testResult.ok ? 'ok' : 'failed'}`}>
              {testResult.message}
            </span>
          )}
          <span className="spacer" />
          {(hasKey || settings.url || settings.username || settings.enabled) && (
            <Button variant="destructive" onClick={this._onRemove}>
              {localized('Remove')}
            </Button>
          )}
        </div>
        {settings.enabled && !capability.available && capability.reason && (
          <div className="integrations-status">{capability.reason}.</div>
        )}
        <PrivacyNote kind="network">
          {privacyNote(kind, settings, hasKey || !!keyInput.trim())}
        </PrivacyNote>
      </>
    );
  }

  render() {
    const { kind } = this.props;
    const { settings, testResult, open } = this.state;
    const look = LOOKS[kind];
    let status: [StatusKind, string] = ['off', localized('Off')];
    if (testResult && !testResult.ok) status = ['bad', localized('Error')];
    else if (settings.enabled) status = ['ok', localized('On')];
    return (
      <ProviderCard
        logo={<ProviderLogo {...look.logo(settings.provider)} />}
        title={look.title()}
        subtitle={subtitle(kind, settings, this.state.hasKey)}
        status={<StatusPill kind={status[0]}>{status[1]}</StatusPill>}
        actions={
          !open &&
          !settings.enabled && (
            <Button variant="outline" onClick={() => this.setState({ open: true })}>
              {localized('Set up')}
            </Button>
          )
        }
        open={open}
        onToggle={(next) => this.setState({ open: next })}
      >
        {this._renderBody()}
      </ProviderCard>
    );
  }
}

export default function PreferencesIntegrations() {
  return (
    <SettingsPage
      className="preferences-integrations"
      title={localized('Integrations')}
      description={localized(
        'Services Tern can call for translation, grammar and contact details. API keys are stored encrypted, like your account passwords.'
      )}
    >
      <SettingsSection>
        <IntegrationSection kind="translation" />
        <IntegrationSection kind="grammar" />
        <IntegrationSection kind="enrichment" />
      </SettingsSection>
      <SettingsNote>
        {localizedReactFragment(
          'AI models are set up in %@. Apps that use your mail, like Claude, are in %@.',
          <a key="ai" onClick={() => Actions.switchPreferencesTab('AI')}>
            {localized('AI')}
          </a>,
          <a key="mcp" onClick={() => Actions.switchPreferencesTab('Agents')}>
            {localized('MCP')}
          </a>
        )}
      </SettingsNote>
    </SettingsPage>
  );
}

PreferencesIntegrations.displayName = 'PreferencesIntegrations';
