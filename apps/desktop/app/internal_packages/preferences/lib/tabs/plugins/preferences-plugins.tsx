import React from 'react';
import { localized } from 'mailspring-exports';

import { pluginInstallRecord } from '../../../../../src/plugin-install-record';
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

export class PluginsSection extends React.Component<Props, State> {
  static displayName = 'PluginsSection';

  _mounted = false;

  state: State = { services: null, listError: null, busy: null, plan: null, results: {} };

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
    this.setState({ results: { ...this.state.results, [pkg]: { ok, message } } });
  }

  _onReview = async (pkg: string) => {
    this.setState({ busy: pkg, plan: null });
    try {
      const plan = await planServiceInstall(pkg, this._unitOptions());
      if (this._mounted) this.setState({ plan });
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
          : localized('Service installed (not loaded).')
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
                'First it installs the engine unit %1$@, which runs %2$@ in the background:',
                plan.engine.path,
                plan.engine.bin
              )}
            </p>
            <pre className="plugin-unit">{plan.engine.text}</pre>
          </>
        )}
        <p>{localized('The service unit %@:', plan.unit.path)}</p>
        <pre className="plugin-unit">{plan.unit.text}</pre>
        <div className="plugin-actions">
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

  _renderService(plugin: PluginRow, record: ServiceRecord | undefined) {
    const { busy, plan } = this.state;
    const result = this.state.results[plugin.name];
    const problems = [];
    if (record && !record.unitPresent) problems.push(localized('its unit file is missing'));
    if (record && !record.binUnchanged) problems.push(localized('its binary changed'));
    return (
      <div className="plugin-service">
        {record ? (
          <div className="item">
            <span className="plugin-service-status">
              {localized('Service: %1$@ (SHA-256 %2$@)', record.bin, shortHash(record.sha256))}
              {problems.length > 0 && ` - ${problems.join(', ')}`}
            </span>
            <button
              className="btn btn-small"
              id={`plugin-uninstall-${plugin.name}`}
              disabled={busy === plugin.name}
              onClick={() => this._onUninstall(plugin.name)}
            >
              {localized('Uninstall service')}
            </button>
          </div>
        ) : (
          plan?.package !== plugin.name && (
            <div className="item">
              <span className="plugin-service-status">
                {localized('Has a background service, not installed.')}
              </span>
              <button
                className="btn btn-small"
                id={`plugin-review-${plugin.name}`}
                disabled={busy === plugin.name || !ternBinary()}
                onClick={() => this._onReview(plugin.name)}
              >
                {localized('Install service…')}
              </button>
            </div>
          )
        )}
        {plan?.package === plugin.name && this._renderPlan(plan)}
        {result && (
          <span className={`plugin-result ${result.ok ? 'ok' : 'failed'}`}>{result.message}</span>
        )}
      </div>
    );
  }

  _renderPlugin(plugin: PluginRow) {
    const record = pluginInstallRecord(plugin.name);
    const service = this.state.services?.services.find((s) => s.package === plugin.name);
    return (
      <div key={plugin.name} className="plugin" id={`plugin-${plugin.name}`}>
        <div className="plugin-name">
          {plugin.displayName}
          {plugin.version && <span className="plugin-version"> {plugin.version}</span>}
          {!plugin.active && <span className="plugin-inactive"> ({localized('not active')})</span>}
        </div>
        <div className="plugin-detail">{plugin.directory}</div>
        <div className="plugin-detail">
          {record
            ? localized(
                'Installed from %1$@ on %2$@, SHA-256 %3$@',
                record.source,
                new Date(record.installedAt).toLocaleDateString(),
                shortHash(record.sha256)
              )
            : localized('Not installed through Tern: no source or hash recorded.')}
        </div>
        {(plugin.hasService || service) && this._renderService(plugin, service)}
      </div>
    );
  }

  render() {
    const plugins = this._plugins();
    const listed = new Set(plugins.map((p) => p.name));
    const orphans = (this.state.services?.services || []).filter((s) => !listed.has(s.package));
    return (
      <section className="plugins-section">
        <h6>{localized('Plugins')}</h6>
        <p className="plugins-note">
          {localized(
            'Plugins run as part of Tern with full access to your mail and files. Only install plugins you trust.'
          )}
        </p>
        {!ternBinary() && (
          <p className="plugins-note">
            {localized('Installing services needs the tern command, which is not in this build.')}
          </p>
        )}
        {this.state.listError && <p className="plugin-result failed">{this.state.listError}</p>}
        {plugins.length === 0 ? (
          <p className="plugins-note">{localized('No plugins installed.')}</p>
        ) : (
          plugins.map((p) => this._renderPlugin(p))
        )}
        {orphans.map((s) =>
          this._renderPlugin({
            name: s.package,
            displayName: s.package,
            version: null,
            directory: s.source,
            active: false,
            hasService: true,
          })
        )}
        <div className="plugin-actions">
          <button className="btn" onClick={() => AppEnv.packages.installPackageManually()}>
            {localized('Install a plugin…')}
          </button>
        </div>
      </section>
    );
  }
}

export default class PreferencesPlugins extends React.Component {
  static displayName = 'PreferencesPlugins';

  render() {
    return (
      <div className="container-plugins">
        <PluginsSection />
      </div>
    );
  }
}
