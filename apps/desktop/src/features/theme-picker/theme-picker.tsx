import React from 'react';

import { Flexbox } from '../../components/flexbox';
import { ScrollRegion } from '../../components/scroll-region';
import { localized } from '../../intl/localize';
import {
  AUTOMATIC_THEME_NAME,
  LIGHT_THEME_NAME,
  DARK_THEME_NAME,
} from '../../packages/theme-manager';
import ThemeOption, { toSelector } from './theme-option';
import { RadioGroup } from '../../components/kit/radio-group';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../../components/kit/select';
import { Disposable } from 'event-kit';

// Sort order for built-in themes; community themes not in this list sort last.
const INTERNAL_THEME_ORDER = [
  'ui-less-is-more',
  'ui-taiga',
  'ui-darkside',
  DARK_THEME_NAME,
  LIGHT_THEME_NAME,
  AUTOMATIC_THEME_NAME,
];

function sortThemes<T extends { name: string }>(themes: T[]): T[] {
  return [...themes].sort(
    (a, b) => (INTERNAL_THEME_ORDER.indexOf(a.name) - INTERNAL_THEME_ORDER.indexOf(b.name)) * -1
  );
}

class ThemePicker extends React.Component<
  Record<string, unknown>,
  { themes: any[]; activeTheme: string; lightTheme: string; darkTheme: string }
> {
  static displayName = 'ThemePicker';

  themes = AppEnv.themes;
  disposable?: Disposable;

  constructor(props: Record<string, unknown>) {
    super(props);
    this.state = this._getState();
  }

  componentDidMount() {
    this.disposable = this.themes.onDidChangeActiveThemes(() => {
      this.setState(this._getState());
    });
  }

  componentWillUnmount() {
    if (this.disposable) this.disposable.dispose();
  }

  _getState() {
    return {
      themes: this.themes.getAvailableThemes(),
      activeTheme: this.themes.getActiveThemeSetting().name,
      lightTheme: this.themes.getConfiguredLightThemeName(),
      darkTheme: this.themes.getConfiguredDarkThemeName(),
    };
  }

  _setActiveTheme(theme: string) {
    const prevActiveTheme = this.state.activeTheme;
    this.themes.setActiveTheme(theme);
    this._rewriteIFrame(prevActiveTheme, theme);
  }

  _setLightTheme(themeName: string) {
    this.themes.setLightTheme(themeName);
    this.setState({ lightTheme: themeName });
  }

  _setDarkTheme(themeName: string) {
    this.themes.setDarkTheme(themeName);
    this.setState({ darkTheme: themeName });
  }

  _rewriteIFrame(prevActiveTheme: string, activeTheme: string) {
    const activeFrame = document.querySelector(`.${toSelector(activeTheme)}`) as HTMLIFrameElement;
    const prevActiveFrame = document.querySelector(
      `.${toSelector(prevActiveTheme)}`
    ) as HTMLIFrameElement;

    if (prevActiveFrame) {
      // Found in this document, so same-origin and attached: it has a document.
      const prevActiveDoc = prevActiveFrame.contentDocument!;
      const prevActiveElement = prevActiveDoc.querySelector('.theme-option.active-true');
      if (prevActiveElement) prevActiveElement.className = 'theme-option active-false';
    }

    if (activeFrame) {
      const activeDoc = activeFrame.contentDocument!;
      const activeElement = activeDoc.querySelector('.theme-option.active-false');
      if (activeElement) activeElement.className = 'theme-option active-true';
    }
  }

  _renderThemeOptions() {
    return sortThemes(this.state.themes).map((theme) => (
      <ThemeOption key={theme.name} theme={theme} active={this.state.activeTheme === theme.name} />
    ));
  }

  _renderThemeSelect(
    themes: { name: string; displayName: string }[],
    value: string,
    label: string,
    onChange: (name: string) => void
  ) {
    return (
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger size="sm" aria-label={label}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {themes.map((t) => (
            <SelectItem key={t.name} value={t.name}>
              {t.displayName}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    );
  }

  _renderAutoSlots() {
    if (this.state.activeTheme !== AUTOMATIC_THEME_NAME) return null;

    const sorted = sortThemes(this.state.themes.filter((t) => t.name !== AUTOMATIC_THEME_NAME));

    return (
      <div className="auto-theme-selectors">
        <div className="auto-theme-selector">
          <label>
            <span className="auto-theme-icon">☀</span>
            {localized('When light')}
          </label>
          {this._renderThemeSelect(sorted, this.state.lightTheme, localized('When light'), (name) =>
            this._setLightTheme(name)
          )}
        </div>
        <div className="auto-theme-selector">
          <label>
            <span className="auto-theme-icon">☾</span>
            {localized('When dark')}
          </label>
          {this._renderThemeSelect(sorted, this.state.darkTheme, localized('When dark'), (name) =>
            this._setDarkTheme(name)
          )}
        </div>
      </div>
    );
  }

  render() {
    return (
      <div className="theme-picker">
        <Flexbox direction="column">
          <h4 style={{ color: '#434648' }}>{localized('Themes')}</h4>
          <div style={{ color: 'rgba(35, 31, 32, 0.5)', fontSize: '12px' }}>
            {localized('Click any theme to apply:')}
          </div>
          <ScrollRegion style={{ margin: '10px 5px 0 5px', height: '300px' }}>
            {this._renderAutoSlots()}
            <RadioGroup
              className="theme-options"
              orientation="horizontal"
              aria-label={localized('Themes')}
              value={this.state.activeTheme}
              onValueChange={(name) => this._setActiveTheme(name)}
            >
              {this._renderThemeOptions()}
            </RadioGroup>
          </ScrollRegion>
          <div className="create-theme">
            <a
              href="https://github.com/ternmail/ternmail/blob/main/docs/desktop/theme-tokens.md"
              style={{ color: '#3187e1' }}
            >
              {localized('Create a Theme')}
            </a>
          </div>
        </Flexbox>
      </div>
    );
  }
}

export default ThemePicker;
