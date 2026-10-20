import React from 'react';
import { localized } from 'mailspring-exports';
import {
  SettingsPage,
  SettingsSection,
  SettingsRow,
  SettingsField,
  StatusPill,
  ProviderCard,
  ProviderLogo,
  PrivacyNote,
} from 'mailspring-component-kit';

import { pluginInstallRecord } from '../../../../packages/install-record';
import {
  ServiceList,
  ServicePlan,
  ServiceRecord,
  UnitOptions,
  installService,
  listServices,
  planServiceInstall,
  ternBinary,
  uninstallService,
} from './tern-cli';

// Preferences > Plugins: the plugins installed in the profile, where each came from and its
// hash, and their service programs (docs/desktop-client-plan.md T19, TD29). Installing a
// service shows the exact binary, its hash and the units tern will write before anything is
// written; the install then refuses a binary whose hash changed in between.

interface PluginRow {
  name: string;
  displayName: string;
  version: string | null;
  directory: string;
  active: boolean;
  hasService: boolean;
}

interface Props extends UnitOptions {
  /** The plugins to list; defaults to the third-party ones the app has found. */
  plugins?: PluginRow[];
}

interface State {
  services: ServiceList | null;
  listError: string | null;
  busy: string | null;
  plan: ServicePlan | null;
  results: { [pkg: string]: { ok: boolean; message: string } };
  open: { [pkg: string]: boolean };
}

export function thirdPartyPlugins(): PluginRow[] {
  const { resourcePath } = AppEnv.getLoadSettings();
  return AppEnv.packages
    .getAvailablePackages()
    .filter((pkg) => !pkg.directory.startsWith(resourcePath))
    .map((pkg) => ({
      name: pkg.name,
      displayName: pkg.displayName,
      version: pkg.version(),
      directory: pkg.directory,
      active: AppEnv.packages.isPackageActive(pkg.name),
      hasService: !!pkg.serviceManifest(),
    }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

function shortHash(hash: string) {
  return hash.slice(0, 12);
}

const LOGO_COLORS = ['#e8467f', '#5d6472', '#3b7dd8', '#10a37f', '#c96442', '#7b2ff7', '#d08a00'];

function logoFor(name: string) {
  let n = 0;
  for (const c of name) n = (n * 31 + c.charCodeAt(0)) >>> 0;
  return <ProviderLogo text={name.slice(0, 1).toUpperCase()} color={LOGO_COLORS[n % 7]} />;
}

export class PluginsSection extends React.Component<Props, State> {
  static displayName = 'PluginsSection';

  _mounted = false;

  state: State = {
    services: null,
    listError: null,
    busy: null,
    plan: null,
    results: {},
    open: {},
  };

  componentDidMount() {
    this._mounted = true;
    this._refresh();
  }

  componentWillUnmount() {
    this._mounted = false;
  }

  _plugins() {
    return this.props.plugins || thirdPartyPlugins();
  }

  _unitOptions(): UnitOptions {
    return { unitDir: this.props.unitDir };
  }

  _refresh = async () => {
    if (!ternBinary()) return;
    try {
      const services = await listServices();
      if (this._mounted) this.setState({ services, listError: null });
    } catch (err) {
      if (this._mounted) this.setState({ listError: err.message });
    }
  };

  _result(pkg: string, ok: boolean, message: string) {
    if (!this._mounted) return;
    this.setState({
      results: { ...this.state.results, [pkg]: { ok, message } },
      open: { ...this.state.open, [pkg]: true },
    });
  }

  _onReview = async (pkg: string) => {
    this.setState({ busy: pkg, plan: null });
    try {
      const plan = await planServiceInstall(pkg, this._unitOptions());
      if (this._mounted) this.setState({ plan, open: { ...this.state.open, [pkg]: true } });
    } catch (err) {
      this._result(pkg, false, err.message);
    } finally {
      if (this._mounted) this.setState({ busy: null });
    }
  };

  _onInstall = async () => {
    const { plan } = this.state;
    if (!plan) return;
    this.setState({ busy: plan.package });
    try {
      const done = await installService(plan, this._unitOptions());
      this._result(
        plan.package,
        true,
        done.load
          ? localized('Service installed and started.')
          : localized('Service installed. It starts the next time you log in.')
      );
      if (this._mounted) this.setState({ plan: null });
    } catch (err) {
      this._result(plan.package, false, err.message);
    } finally {
      if (this._mounted) this.setState({ busy: null });
    }
    await this._refresh();
  };

  _onUninstall = async (pkg: string) => {
    this.setState({ busy: pkg });
    try {
      await uninstallService(pkg, this._unitOptions());
      this._result(pkg, true, localized('Service removed.'));
    } catch (err) {
      this._result(pkg, false, err.message);
    } finally {
      if (this._mounted) this.setState({ busy: null });
    }
    await this._refresh();
  };

  _renderPlan(plan: ServicePlan) {
    const busy = this.state.busy === plan.package;
    return (
      <div className="plugin-service-plan" id={`plugin-plan-${plan.package}`}>
        <p>
          {localized(
            'Tern will copy %1$@ (SHA-256 %2$@) to %3$@ and run it whenever you are logged in, also while Tern is closed. It has the same access to your mail as Tern.',
            plan.bin,
            plan.sha256,
            plan.installedBin
          )}
        </p>
        {!plan.engine.installed && (
          <>
            <p>
              {localized(
                'So that it can reach your mail while Tern is closed, Tern first sets up %1$@ to run %2$@ in the background:',
                plan.engine.path,
                plan.engine.bin
              )}
            </p>
            <pre className="plugin-unit">{plan.engine.text}</pre>
          </>
        )}
        <p>{localized('The service itself is set up in %@:', plan.unit.path)}</p>
        <pre className="plugin-unit">{plan.unit.text}</pre>
        <div className="settings-actions">
          <button
            className="btn btn-emphasis"
            id={`plugin-install-${plan.package}`}
            disabled={busy}
            onClick={this._onInstall}
          >
            {localized('Install service')}
          </button>
          <button className="btn" disabled={busy} onClick={() => this.setState({ plan: null })}>
            {localized('Cancel')}
          </button>
        </div>
      </div>
    );
  }

  _serviceProblems(record: ServiceRecord | undefined) {
    const problems = [];
    if (record && !record.unitPresent) problems.push(localized('its unit file is missing'));
    if (record && !record.binUnchanged) problems.push(localized('its program changed'));
    return problems;
  }

  _serviceButton(name: string, record: ServiceRecord | undefined) {
    const { busy, plan } = this.state;
    if (record) {
      return (
        <button
          className="btn"
          id={`plugin-uninstall-${name}`}
          disabled={busy === name}
          onClick={() => this._onUninstall(name)}
        >
          {localized('Remove service')}
        </button>
      );
    }
    if (plan?.package === name) return null;
    return (
      <button
        className="btn"
        id={`plugin-review-${name}`}
        disabled={busy === name || !ternBinary()}
        onClick={() => this._onReview(name)}
      >
        {localized('Set up service…')}
      </button>
    );
  }

  _renderPlugin(plugin: PluginRow) {
    const record = pluginInstallRecord(plugin.name);
    const service = this.state.services?.services.find((s) => s.package === plugin.name);
    const hasService = plugin.hasService || !!service;
    const problems = this._serviceProblems(service);
    const result = this.state.results[plugin.name];
    const { plan } = this.state;

    const subtitle = [plugin.version && localized('Version %@', plugin.version)];
    if (hasService) {
      subtitle.push(
        service
          ? localized('background service installed')
          : localized('background service not set up')
      );
    }

    let status: React.ReactNode = null;
    if (problems.length) {
      status = <StatusPill kind="warn">{localized('Service needs attention')}</StatusPill>;
    } else if (!plugin.active) {
      status = <StatusPill kind="off">{localized('Not active')}</StatusPill>;
    }

    return (
      <ProviderCard
        key={plugin.name}
        logo={logoFor(plugin.displayName || plugin.name)}
        title={plugin.displayName}
        subtitle={subtitle.filter(Boolean).join(' · ')}
        status={status}
        actions={hasService && this._serviceButton(plugin.name, service)}
        open={!!this.state.open[plugin.name]}
        onToggle={(open) => this.setState({ open: { ...this.state.open, [plugin.name]: open } })}
      >
        <div id={`plugin-${plugin.name}`}>
          <SettingsField label={localized('Folder')}>
            <span className="plugin-detail">{plugin.directory}</span>
          </SettingsField>
          <SettingsField label={localized('Installed from')}>
            <span className="plugin-detail">
              {record
                ? localized(
                    '%1$@ on %2$@, SHA-256 %3$@',
                    record.source,
                    new Date(record.installedAt).toLocaleDateString(),
                    shortHash(record.sha256)
                  )
                : localized('Not installed through Tern: no source or hash recorded.')}
            </span>
          </SettingsField>
          {service && (
            <SettingsField label={localized('Service')}>
              <span className="plugin-detail">
                {localized('%1$@ (SHA-256 %2$@)', service.bin, shortHash(service.sha256))}
                {problems.length > 0 && ` - ${problems.join(', ')}`}
              </span>
            </SettingsField>
          )}
          {plan?.package === plugin.name && this._renderPlan(plan)}
          {result && (
            <div className={`plugin-result ${result.ok ? 'ok' : 'failed'}`}>{result.message}</div>
          )}
        </div>
      </ProviderCard>
    );
  }

  _renderOrphan(record: ServiceRecord) {
    const result = this.state.results[record.package];
    return (
      <SettingsRow
        key={record.package}
        label={localized('%@ (plugin removed)', record.package)}
        description={
          result ? (
            <span className={`plugin-result ${result.ok ? 'ok' : 'failed'}`}>{result.message}</span>
          ) : (
            localized('Its service still runs %@.', record.bin)
          )
        }
      >
        {this._serviceButton(record.package, record)}
      </SettingsRow>
    );
  }

  render() {
    const plugins = this._plugins();
    const listed = new Set(plugins.map((p) => p.name));
    const orphans = (this.state.services?.services || []).filter((s) => !listed.has(s.package));
    const tern = !!ternBinary();
    return (
      <div className="plugins-section">
        <SettingsSection
          note={
            <a onClick={() => AppEnv.packages.installPackageManually()}>
              {localized('Install from a folder…')}
            </a>
          }
        >
          {plugins.length === 0 ? (
            <SettingsRow
              label={localized('No plugins installed')}
              description={localized('Plugins you install show up here.')}
            />
          ) : (
            plugins.map((p) => this._renderPlugin(p))
          )}
        </SettingsSection>
        <PrivacyNote kind="warn">
          {localized(
            'Plugins run inside Tern and can read all of your mail and files. Only install plugins from people you trust.'
          )}
        </PrivacyNote>

        <SettingsSection title={localized('Background services')}>
          <SettingsRow
            label={localized('Some plugins run a small program in the background')}
            description={
              tern
                ? localized(
                    'It keeps running while Tern is closed, with the same access to your mail as Tern. Set one up from its plugin above.'
                  )
                : localized(
                    'This copy of Tern can’t set them up because it doesn’t include the tern command. Install Tern from the official download to use them.'
                  )
            }
          >
            {!tern && <StatusPill kind="off">{localized('Not available')}</StatusPill>}
          </SettingsRow>
          {this.state.listError && (
            <SettingsRow
              label={localized('Tern couldn’t list the services')}
              description={<span className="plugin-result failed">{this.state.listError}</span>}
            >
              <button className="btn" onClick={this._refresh}>
                {localized('Try again')}
              </button>
            </SettingsRow>
          )}
          {orphans.map((s) => this._renderOrphan(s))}
        </SettingsSection>
      </div>
    );
  }
}

export default function PreferencesPlugins() {
  return (
    <SettingsPage
      title={localized('Plugins')}
      description={localized('Add features to Tern.')}
      className="preferences-plugins"
    >
      <PluginsSection />
    </SettingsPage>
  );
}

PreferencesPlugins.displayName = 'PreferencesPlugins';
