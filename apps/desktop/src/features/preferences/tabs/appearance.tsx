/* eslint global-require: 0 */
import React from 'react';
import classnames from 'classnames';
import { LabelColorizer } from '../../../components/mail-label';
import {
  SettingsPage,
  SettingsSection,
  SettingsRow,
  SegmentedControl,
  ConfigSwitchRow,
  ConfigSelectRow,
} from '../../../components/settings';
import { Button } from '../../../components/kit/button';
import { RadioGroup, RadioGroupItem } from '../../../components/kit/radio-group';
import { SettingsSelect } from '../../../components/settings-select';
import * as Actions from '../../../data/actions';
import { localized } from '../../../intl/localize';
import { isWaylandSession } from '../../../platform/linux-desktop';
import { App } from '../../../platform/app-api';
import { LABEL_COLOR_SCHEMES, LABEL_COLOR_SCHEME_NAMES } from '../../../data/stores/label-color';
import { themePreviewColors } from '../../../packages/theme-preview';
import {
  AUTOMATIC_THEME_NAME,
  LIGHT_THEME_NAME,
  DARK_THEME_NAME,
} from '../../../packages/theme-manager';

const THEME_ORDER = [
  LIGHT_THEME_NAME,
  DARK_THEME_NAME,
  AUTOMATIC_THEME_NAME,
  'ui-less-is-more',
  'ui-taiga',
  'ui-darkside',
];

interface ThemePackage {
  name: string;
  displayName: string;
  getStylesheetsPath: () => string;
}

interface ThemeColors {
  sidebar: string;
  content: string;
  accent: string;
  text: string;
}

const previewColors = new Map<string, ThemeColors | null>();

/** A theme's own colours for its tile (src/packages/theme-preview.ts). */
function colorsForTheme(theme: ThemePackage): ThemeColors | null {
  // Entries are colours or null, never undefined.
  const cached = previewColors.get(theme.name);
  if (cached !== undefined) return cached;
  let colors: ThemeColors | null = null;
  try {
    const { sidebar, content, accent, text } = themePreviewColors(theme);
    if (sidebar && content && accent && text) colors = { sidebar, content, accent, text };
  } catch (err) {
    console.warn(`Could not preview the theme ${theme.name}: ${(err as Error).message}`);
  }
  previewColors.set(theme.name, colors);
  return colors;
}

function ThemeMiniature({ colors }: { colors: ThemeColors | null }) {
  if (!colors) return <div className="theme-miniature blank" />;
  return (
    <div className="theme-miniature" style={{ background: colors.content }}>
      <div className="sidebar" style={{ background: colors.sidebar }} />
      <div className="lines">
        <i style={{ background: colors.accent }} />
        <i style={{ background: colors.text }} />
        <i style={{ background: colors.text }} />
      </div>
    </div>
  );
}

function themeLabel(theme: ThemePackage) {
  if (theme.name === LIGHT_THEME_NAME) return localized('Light');
  if (theme.name === DARK_THEME_NAME) return localized('Dark');
  if (theme.name === AUTOMATIC_THEME_NAME) return localized('Match system');
  return theme.displayName || theme.name;
}

class ThemeChooser extends React.Component<
  Record<string, unknown>,
  { themes: ThemePackage[]; colors: { [name: string]: ThemeColors | null } }
> {
  static displayName = 'ThemeChooser';

  _mounted = false;
  _timer: NodeJS.Timeout | null = null;

  constructor(props: Record<string, unknown>) {
    super(props);
    const themes: ThemePackage[] = AppEnv.themes.getAvailableThemes();
    const rank = (t: ThemePackage) =>
      THEME_ORDER.includes(t.name) ? THEME_ORDER.indexOf(t.name) : THEME_ORDER.length;
    themes.sort((a, b) => rank(a) - rank(b));
    const colors: { [name: string]: ThemeColors | null } = {};
    for (const t of themes) {
      const cached = previewColors.get(t.name);
      if (cached !== undefined) colors[t.name] = cached;
    }
    this.state = { themes, colors };
  }

  componentDidMount() {
    this._mounted = true;
    this._compileNext();
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._timer) clearTimeout(this._timer);
  }

  // One theme per tick: compiling a LESS-variables theme's colours the first time takes long
  // enough to drop frames.
  _compileNext = () => {
    const next = this.state.themes.find(
      (t) => t.name !== AUTOMATIC_THEME_NAME && !(t.name in this.state.colors)
    );
    if (!next || !this._mounted) return;
    this._timer = setTimeout(() => {
      if (!this._mounted) return;
      const colors = colorsForTheme(next);
      this.setState((s) => ({ colors: { ...s.colors, [next.name]: colors } }), this._compileNext);
    }, 0);
  };

  _renderTile(theme: ThemePackage, active: string) {
    const { colors } = this.state;
    let preview: React.ReactNode;
    if (theme.name === AUTOMATIC_THEME_NAME) {
      preview = (
        <div className="theme-miniature split">
          <ThemeMiniature colors={colors[AppEnv.themes.getConfiguredLightThemeName()]} />
          <ThemeMiniature colors={colors[AppEnv.themes.getConfiguredDarkThemeName()]} />
        </div>
      );
    } else {
      preview = <ThemeMiniature colors={colors[theme.name]} />;
    }
    return (
      <RadioGroupItem
        key={theme.name}
        value={theme.name}
        className={classnames('theme-tile', { on: active === theme.name })}
      >
        {preview}
        <span className="theme-tile-name">{themeLabel(theme)}</span>
      </RadioGroupItem>
    );
  }

  _renderSystemThemeSelect(label: string, value: string, onChange: (name: string) => void) {
    const choices = this.state.themes.filter((t) => t.name !== AUTOMATIC_THEME_NAME);
    return (
      <SettingsRow label={label}>
        <SettingsSelect
          aria-label={label}
          value={value}
          onChange={onChange}
          options={choices.map((t): [string, string] => [t.name, themeLabel(t)])}
        />
      </SettingsRow>
    );
  }

  render() {
    const active = AppEnv.themes.getActiveThemeSetting().name;
    return (
      <>
        <RadioGroup
          className="theme-tiles"
          aria-label={localized('Theme')}
          value={active}
          onValueChange={(name) => AppEnv.themes.setActiveTheme(name)}
        >
          {this.state.themes.map((theme) => this._renderTile(theme, active))}
        </RadioGroup>
        {active === AUTOMATIC_THEME_NAME && (
          <>
            {this._renderSystemThemeSelect(
              localized('When the system is light'),
              AppEnv.themes.getConfiguredLightThemeName(),
              (name) => AppEnv.themes.setLightTheme(name)
            )}
            {this._renderSystemThemeSelect(
              localized('When the system is dark'),
              AppEnv.themes.getConfiguredDarkThemeName(),
              (name) => AppEnv.themes.setDarkTheme(name)
            )}
          </>
        )}
      </>
    );
  }
}

type LayoutMode = 'list' | 'split' | 'splitVertical';

/** Switching layout pops back to the mailbox, so the choice waits for Apply. */
class ReadingPaneRow extends React.Component<Record<string, unknown>, { value: LayoutMode }> {
  static displayName = 'ReadingPaneRow';

  state = { value: AppEnv.config.get('core.workspace.mode') as LayoutMode };

  _onApply = () => {
    // The command carries `off` because that is its name while the mode is not the current one.
    AppEnv.commands.dispatch(`navigation:${this.state.value}-mode-off`);
  };

  render() {
    const changed = this.state.value !== AppEnv.config.get('core.workspace.mode');
    return (
      <SettingsRow
        label={localized('Reading pane')}
        description={changed ? localized('Applying it takes you back to your mail.') : undefined}
      >
        <SegmentedControl<LayoutMode>
          label={localized('Reading pane')}
          value={this.state.value}
          options={[
            ['list', localized('Off')],
            ['split', localized('Right')],
            ['splitVertical', localized('Below')],
          ]}
          onChange={(value) => this.setState({ value })}
        />
        {changed && (
          <Button variant="default" onClick={this._onApply}>
            {localized('Apply')}
          </Button>
        )}
      </SettingsRow>
    );
  }
}

const LABEL_SCHEME_NAMES = () => ({
  automatic: localized('Automatic'),
  bright: localized('Bright'),
  pastel: localized('Pastel'),
  muted: localized('Muted'),
  earth: localized('Earth'),
});

function LabelColorSchemeRows() {
  const keyPath = 'core.appearance.labelColorScheme';
  const value = AppEnv.config.get(keyPath) || 'automatic';
  const names = LABEL_SCHEME_NAMES();
  const hues = Array.from({ length: 12 }, (_, i) => `hsl(${i * 30}, 62%, 47%)`);
  return (
    <RadioGroup
      className="label-scheme-rows"
      aria-label={localized('Label colours')}
      value={value}
      onValueChange={(name) => AppEnv.config.set(keyPath, name)}
    >
      {LABEL_COLOR_SCHEME_NAMES.map((name) => (
        <RadioGroupItem
          key={name}
          value={name}
          className={classnames('settings-row label-scheme-row', { on: value === name })}
        >
          <div className="settings-row-label">
            <span className="label-scheme-check" aria-hidden />
            <span className="settings-row-title">{names[name]}</span>
          </div>
          <div className="label-scheme-swatches" aria-hidden>
            {(name === 'automatic' ? hues : LABEL_COLOR_SCHEMES[name]).map((color) => (
              <i key={color} style={{ backgroundColor: LabelColorizer.dotColorFor(color) }} />
            ))}
          </div>
        </RadioGroupItem>
      ))}
    </RadioGroup>
  );
}

function InterfaceScaleRow() {
  const keyPath = 'core.workspace.interfaceZoom';
  const saved = Number(AppEnv.config.get(keyPath)) || 1;
  // Zooming rescales the slider under the pointer, so applying it mid-drag makes the thumb
  // jump. Hold the value while dragging and apply it on release.
  const [dragging, setDragging] = React.useState<number | null>(null);
  const value = dragging ?? saved;
  const commit = () => {
    if (dragging !== null && dragging !== saved) AppEnv.config.set(keyPath, dragging);
    setDragging(null);
  };
  return (
    <SettingsRow
      label={localized('Interface size')}
      description={localized(
        'Scales text, icons and dividers in Tern. Mail you send keeps its own size. Very small sizes make some controls hard to click.'
      )}
      className="interface-scale-row"
    >
      <span className="scale-glyph small" aria-hidden>
        A
      </span>
      <input
        id="interface-zoom-slider"
        type="range"
        min={0.8}
        max={1.4}
        step={0.05}
        value={value}
        aria-label={localized('Interface size')}
        onChange={(e) => setDragging(Number(e.target.value))}
        onPointerUp={commit}
        onKeyUp={commit}
        onBlur={commit}
      />
      <span className="scale-glyph large" aria-hidden>
        A
      </span>
      <span className="scale-value">{`${Math.round(value * 100)}%`}</span>
    </SettingsRow>
  );
}

function WindowControlsRows() {
  if (process.platform !== 'linux') return null;
  const keyPath = 'core.workspace.menubarStyle';
  const options: [string, string][] = [
    ['default', localized('System title bar and menu bar')],
    ['autohide', localized('System title bar, menu bar hidden until you press Alt')],
    ['hamburger', localized("Tern's own title bar with a menu button")],
  ];
  const value = AppEnv.config.get(keyPath);
  let description = localized('Relaunch Tern to apply.');
  if (value === 'default' && isWaylandSession()) {
    description = localized(
      'The menu bar may not show on Wayland; Tern adds a menu button instead. Relaunch Tern to apply.'
    );
  } else if (value === 'autohide') {
    description = localized('Needs a window manager that supports it. Relaunch Tern to apply.');
  }
  return (
    <SettingsSection title={localized('Window')}>
      <ConfigSelectRow
        keyPath={keyPath}
        label={localized('Title bar and menus')}
        description={description}
        options={options}
      />
      <SettingsRow label={localized('Relaunch Tern')}>
        <Button
          variant="outline"
          onClick={() => {
            App.relaunch();
            App.quit();
          }}
        >
          {localized('Relaunch')}
        </Button>
      </SettingsRow>
    </SettingsSection>
  );
}

function TrayIconRows() {
  const isMac = process.platform === 'darwin';
  return (
    <SettingsSection title={isMac ? localized('Menu bar icon') : localized('Tray icon')}>
      <ConfigSelectRow
        keyPath="core.workspace.trayIconStyle"
        label={localized('Unread mail')}
        description={localized('How the icon shows that you have unread mail.')}
        options={[
          ['blue', localized('Blue for new and unread mail')],
          ['red', localized('Red for new mail, blue for older unread mail')],
          ['none', localized("Don't show unread mail")],
        ]}
      />
      {process.platform === 'linux' && (
        <ConfigSelectRow
          keyPath="core.workspace.traySystemTheme"
          label={localized('Tray background')}
          description={localized(
            'Pick the one that matches your panel so the icon stays visible. Automatic follows the system theme and assumes a dark panel on GNOME and Unity.'
          )}
          options={[
            ['automatic', localized('Automatic')],
            ['light', localized('Light, use dark icons')],
            ['dark', localized('Dark, use light icons')],
          ]}
        />
      )}
    </SettingsSection>
  );
}

export default function PreferencesAppearance() {
  return (
    <SettingsPage
      className="preferences-appearance"
      title={localized('Appearance')}
      description={localized('Theme, layout and colours.')}
    >
      <SettingsSection
        title={localized('Theme')}
        note={
          <>
            {localized('More themes come from plugins.')}{' '}
            <a onClick={() => Actions.switchPreferencesTab('Plugins')}>
              {localized('Browse plugins')}
            </a>
          </>
        }
      >
        <ThemeChooser />
        <ConfigSwitchRow
          keyPath="core.appearance.useSystemAccent"
          label={localized('Use the system accent colour')}
          description={localized('Buttons and highlights follow the colour set in your system.')}
        />
        <ConfigSelectRow
          keyPath="core.appearance.emailRenderMode"
          label={localized('Message colours')}
          description={localized('Dark backgrounds can change the colours senders chose.')}
          options={[
            ['auto', localized('Dark when the theme is dark')],
            ['dark', localized('Always dark')],
            ['original', localized("Sender's colours")],
          ]}
        />
      </SettingsSection>

      <SettingsSection title={localized('Layout')}>
        <ReadingPaneRow />
        <InterfaceScaleRow />
      </SettingsSection>

      <SettingsSection
        title={localized('Label colours')}
        note={localized('Right-click a label in the sidebar to give it a colour of its own.')}
      >
        <LabelColorSchemeRows />
      </SettingsSection>

      <WindowControlsRows />
      <TrayIconRows />
    </SettingsPage>
  );
}

PreferencesAppearance.displayName = 'PreferencesAppearance';
