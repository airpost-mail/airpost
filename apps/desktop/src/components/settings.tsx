import React from 'react';
import classNames from 'classnames';
import { localized } from '../intl';
import Switch from './switch';

/* Public: The building blocks of a Preferences page: a page header, titled sections of
   cards, rows with a label and description on the left and a control on the right, and
   provider cards that open to show their fields. Plugins use them so their tabs look
   like the built-in ones.

   ```
   <SettingsPage title="Snooze" description="When snoozed mail comes back.">
     <SettingsSection title="Inbox">
       <ConfigSwitchRow keyPath="core.notifications.unsnoozeToTop" label="Move it to the top" />
     </SettingsSection>
   </SettingsPage>
   ```
*/

export type StatusKind = 'ok' | 'warn' | 'bad' | 'off' | 'server';

export function SettingsPage(props: {
  title: string;
  description?: React.ReactNode;
  icon?: React.ReactNode;
  className?: string;
  children?: React.ReactNode;
}) {
  return (
    <div className={classNames('settings-page', props.className)}>
      {(props.title || props.description) && (
        <header className="settings-page-header">
          {props.icon}
          <div>
            <h1>{props.title}</h1>
            {props.description && <p>{props.description}</p>}
          </div>
        </header>
      )}
      {props.children}
    </div>
  );
}

export function SettingsSection(props: {
  title?: React.ReactNode;
  aside?: React.ReactNode;
  note?: React.ReactNode;
  danger?: boolean;
  bare?: boolean;
  className?: string;
  children?: React.ReactNode;
}) {
  return (
    <section className={classNames('settings-section', props.className)}>
      {props.title && (
        <h2 className="settings-section-title">
          {props.title}
          {props.aside}
        </h2>
      )}
      {props.bare ? (
        props.children
      ) : (
        <SettingsCard danger={props.danger}>{props.children}</SettingsCard>
      )}
      {props.note && <SettingsNote>{props.note}</SettingsNote>}
    </section>
  );
}

export function SettingsCard(props: {
  danger?: boolean;
  className?: string;
  children?: React.ReactNode;
}) {
  return (
    <div className={classNames('settings-card', props.className, { danger: props.danger })}>
      {props.children}
    </div>
  );
}

export function SettingsNote(props: { children?: React.ReactNode }) {
  return <div className="settings-note">{props.children}</div>;
}

export function SettingsRow(props: {
  label: React.ReactNode;
  description?: React.ReactNode;
  disabled?: boolean;
  className?: string;
  children?: React.ReactNode;
}) {
  return (
    <div className={classNames('settings-row', props.className, { disabled: props.disabled })}>
      <div className="settings-row-label">
        <div className="settings-row-title">{props.label}</div>
        {props.description && <div className="settings-row-description">{props.description}</div>}
      </div>
      <div className="settings-row-control">{props.children}</div>
    </div>
  );
}

export function StatusPill(props: { kind: StatusKind; children?: React.ReactNode }) {
  return <span className={`settings-pill ${props.kind}`}>{props.children}</span>;
}

/** Marks a setting that only works with a Tern server, until the desktop can sign in to one. */
export function NeedsServerPill() {
  return <StatusPill kind="server">{localized('Needs a Tern server')}</StatusPill>;
}

export function SegmentedControl<T extends string | number | boolean>(props: {
  value: T;
  options: [T, string][];
  onChange: (value: T) => void;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <div className="settings-segmented" role="radiogroup" aria-label={props.label}>
      {props.options.map(([value, label]) => (
        <button
          key={String(value)}
          role="radio"
          aria-checked={value === props.value}
          className={classNames({ on: value === props.value })}
          disabled={props.disabled}
          onClick={() => props.onChange(value)}
        >
          {label}
        </button>
      ))}
    </div>
  );
}

export function SettingsSwitch(props: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <Switch
      className={classNames('settings-switch', { disabled: props.disabled })}
      checked={props.checked}
      label={props.label}
      onChange={() => !props.disabled && props.onChange(!props.checked)}
    />
  );
}

type Option = [string | number | boolean, string];

/** The schema's `enum` and `enumLabels` for `keyPath`, as options. */
function schemaOptions(keyPath: string): Option[] {
  const schema = (AppEnv.config.getSchema(keyPath) || {}) as {
    enum?: any[];
    enumLabels?: string[];
  };
  const values: any[] = schema.enum || [];
  const labels: string[] = schema.enumLabels || values.map(String);
  return values.map((v, i) => [v, labels[i]] as Option);
}

/** A switch bound to a boolean config key. `invert` shows on when the key is false. */
export function ConfigSwitchRow(props: {
  keyPath: string;
  label: React.ReactNode;
  description?: React.ReactNode;
  invert?: boolean;
  disabled?: boolean;
  onChange?: (checked: boolean) => void;
}) {
  const raw = !!AppEnv.config.get(props.keyPath);
  const checked = props.invert ? !raw : raw;
  return (
    <SettingsRow label={props.label} description={props.description} disabled={props.disabled}>
      <SettingsSwitch
        checked={checked}
        disabled={props.disabled}
        label={typeof props.label === 'string' ? props.label : undefined}
        onChange={(next) => {
          AppEnv.config.set(props.keyPath, props.invert ? !next : next);
          props.onChange?.(next);
        }}
      />
    </SettingsRow>
  );
}

/** A select bound to a config key; options default to the schema's enum and labels. */
export function ConfigSelectRow(props: {
  keyPath: string;
  label: React.ReactNode;
  description?: React.ReactNode;
  options?: Option[];
  disabled?: boolean;
}) {
  const options = props.options || schemaOptions(props.keyPath);
  const value = AppEnv.config.get(props.keyPath) ?? options[0]?.[0];
  return (
    <SettingsRow label={props.label} description={props.description} disabled={props.disabled}>
      <select
        className="settings-select"
        value={String(value)}
        disabled={props.disabled}
        aria-label={typeof props.label === 'string' ? props.label : undefined}
        onChange={(e) => {
          const picked = options.find(([v]) => String(v) === e.target.value);
          AppEnv.config.set(props.keyPath, picked ? picked[0] : e.target.value);
        }}
      >
        {options.map(([v, label]) => (
          <option key={String(v)} value={String(v)}>
            {label}
          </option>
        ))}
      </select>
    </SettingsRow>
  );
}

/** A segmented control bound to a config key, for two to four short choices. */
export function ConfigSegmentedRow(props: {
  keyPath: string;
  label: React.ReactNode;
  description?: React.ReactNode;
  options?: Option[];
  disabled?: boolean;
}) {
  const options = props.options || schemaOptions(props.keyPath);
  return (
    <SettingsRow label={props.label} description={props.description} disabled={props.disabled}>
      <SegmentedControl
        value={AppEnv.config.get(props.keyPath) ?? options[0]?.[0]}
        options={options}
        disabled={props.disabled}
        label={typeof props.label === 'string' ? props.label : undefined}
        onChange={(v) => AppEnv.config.set(props.keyPath, v)}
      />
    </SettingsRow>
  );
}

/** A note under a provider's fields about what leaves the computer. */
export function PrivacyNote(props: {
  kind?: 'local' | 'network' | 'warn';
  children?: React.ReactNode;
}) {
  return (
    <div className={`settings-privacy ${props.kind || 'network'}`}>
      <span className="settings-privacy-icon" aria-hidden />
      <div>{props.children}</div>
    </div>
  );
}

/** A two-letter square standing in for a provider's or an app's logo. */
export function ProviderLogo(props: { text: string; color: string }) {
  return (
    <div className="settings-provider-logo" style={{ background: props.color }} aria-hidden>
      {props.text}
    </div>
  );
}

/* Public: One provider, app or plugin in a card: logo, name, one line of detail, a status
   and actions. With `children` it opens and closes to show its fields; `open` and
   `onToggle` make it controlled, otherwise it keeps its own state from `defaultOpen`. */
export class ProviderCard extends React.Component<
  {
    logo?: React.ReactNode;
    title: React.ReactNode;
    subtitle?: React.ReactNode;
    status?: React.ReactNode;
    actions?: React.ReactNode;
    open?: boolean;
    defaultOpen?: boolean;
    onToggle?: (open: boolean) => void;
    children?: React.ReactNode;
  },
  { open: boolean }
> {
  static displayName = 'ProviderCard';

  state = { open: !!this.props.defaultOpen };

  _isOpen() {
    return this.props.open !== undefined ? this.props.open : this.state.open;
  }

  _toggle = (e: React.MouseEvent | React.KeyboardEvent) => {
    if ((e.target as HTMLElement).closest('button, input, select, a, [role="switch"]')) return;
    const next = !this._isOpen();
    if (this.props.open === undefined) this.setState({ open: next });
    this.props.onToggle?.(next);
  };

  render() {
    const expandable = React.Children.count(this.props.children) > 0;
    const open = expandable && this._isOpen();
    return (
      <div className={classNames('settings-provider', { open, expandable })}>
        <div
          className="settings-provider-head"
          onClick={expandable ? this._toggle : undefined}
          onKeyDown={
            expandable
              ? (e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    this._toggle(e);
                  }
                }
              : undefined
          }
          role={expandable ? 'button' : undefined}
          aria-expanded={expandable ? open : undefined}
          tabIndex={expandable ? 0 : undefined}
        >
          {this.props.logo}
          <div className="settings-provider-text">
            <div className="settings-provider-title">{this.props.title}</div>
            {this.props.subtitle && (
              <div className="settings-provider-subtitle">{this.props.subtitle}</div>
            )}
          </div>
          <div className="settings-provider-actions">
            {this.props.status}
            {this.props.actions}
            {expandable && <span className="settings-chevron" aria-hidden />}
          </div>
        </div>
        {open && <div className="settings-provider-body">{this.props.children}</div>}
      </div>
    );
  }
}

/** A label and a control on one line inside a provider card's body. */
export function SettingsField(props: {
  label: React.ReactNode;
  htmlFor?: string;
  children?: React.ReactNode;
}) {
  return (
    <div className="settings-field">
      <label htmlFor={props.htmlFor}>{props.label}</label>
      <div>{props.children}</div>
    </div>
  );
}
